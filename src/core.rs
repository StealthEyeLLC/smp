use crate::assets::{ensure_assets, load_manifest};
use crate::firecracker;
use crate::guest;
use crate::model::{
    AssetIdentity, DiskRecord, MachineMode, MachineRecord, MachineState, PublishedPort, TypedError,
    VirtioTransport, MACHINE_SCHEMA_VERSION,
};
use crate::network;
use crate::state::{
    load_machine, remove_machine_dir, safe_remove_file, save_machine, MachineLock, RuntimePaths,
};
use crate::util::{now_unix_ms, run_output, sha256_file, validate_machine_name};
use anyhow::{bail, Result};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct CreateOptions {
    pub name: String,
    pub mode: MachineMode,
    pub transport: VirtioTransport,
    pub vcpu_count: u8,
    pub memory_mib: u32,
    pub rootfs: Option<PathBuf>,
    pub kernel: Option<PathBuf>,
    pub firecracker: Option<PathBuf>,
    pub boot_args: Option<String>,
    pub published_ports: Vec<PublishedPort>,
    pub offline: bool,
}

impl Default for CreateOptions {
    fn default() -> Self {
        Self {
            name: "default".to_owned(),
            mode: MachineMode::Persistent,
            transport: VirtioTransport::Pci,
            vcpu_count: 2,
            memory_mib: 2048,
            rootfs: None,
            kernel: None,
            firecracker: None,
            boot_args: None,
            published_ports: Vec::new(),
            offline: false,
        }
    }
}

fn default_boot_args(root_uuid: &str) -> String {
    format!(
        "root=UUID={root_uuid} rw console=ttyS0 reboot=k panic=1 pci=on net.ifnames=0 systemd.unified_cgroup_hierarchy=1"
    )
}

pub fn create(paths: &RuntimePaths, options: &CreateOptions) -> Result<MachineRecord> {
    validate_machine_name(&options.name)?;
    if options.vcpu_count == 0 || options.vcpu_count > 32 {
        bail!("vcpu count must be between 1 and 32");
    }
    if options.memory_mib < 128 {
        bail!("memory must be at least 128 MiB");
    }
    paths.ensure()?;
    let _lock = MachineLock::acquire(paths, &options.name)?;
    if paths.machine_state_path(&options.name)?.exists() {
        return load_machine(paths, &options.name);
    }

    let manifest = ensure_assets(paths, options.offline)?;
    let machine_dir = paths.machine_dir(&options.name)?;
    fs::create_dir_all(&machine_dir)?;
    let root_path = machine_dir.join("root.ext4");
    let selected_root = options
        .rootfs
        .as_deref()
        .unwrap_or(Path::new(&manifest.rootfs.path));
    let root_identity = custom_or_manifest_identity(
        selected_root,
        options.rootfs.is_some(),
        &manifest.rootfs,
        "operator-rootfs",
    )?;
    guest::create_writable_root(selected_root, &root_path, &options.mode)?;
    let root_uuid = filesystem_uuid(&root_path)?;
    let seed_path = machine_dir.join("seed.ext4");

    let firecracker = custom_or_manifest_identity(
        options
            .firecracker
            .as_deref()
            .unwrap_or(Path::new(&manifest.firecracker.path)),
        options.firecracker.is_some(),
        &manifest.firecracker,
        "operator-firecracker",
    )?;
    let kernel = custom_or_manifest_identity(
        options
            .kernel
            .as_deref()
            .unwrap_or(Path::new(&manifest.kernel.path)),
        options.kernel.is_some(),
        &manifest.kernel,
        "operator-kernel",
    )?;
    let network = network::default_network(&options.name, options.published_ports.clone());
    let now = now_unix_ms();
    let mut record = MachineRecord {
        schema_version: MACHINE_SCHEMA_VERSION,
        name: options.name.clone(),
        architecture: "x86_64".to_owned(),
        mode: options.mode.clone(),
        state: MachineState::Created,
        transport: options.transport.clone(),
        vcpu_count: options.vcpu_count,
        memory_mib: options.memory_mib,
        boot_args: options
            .boot_args
            .clone()
            .unwrap_or_else(|| default_boot_args(&root_uuid)),
        firecracker,
        kernel,
        rootfs_base: root_identity.clone(),
        disks: vec![
            DiskRecord {
                drive_id: "root".to_owned(),
                path: root_path.to_string_lossy().into_owned(),
                logical_size_bytes: fs::metadata(&root_path)?.len(),
                filesystem_uuid: Some(root_uuid),
                writable: true,
                attached: false,
                base_image_sha256: Some(root_identity.sha256.clone()),
            },
            DiskRecord {
                drive_id: "seed".to_owned(),
                path: seed_path.to_string_lossy().into_owned(),
                logical_size_bytes: 0,
                filesystem_uuid: None,
                writable: false,
                attached: false,
                base_image_sha256: None,
            },
        ],
        network,
        ssh_user: "root".to_owned(),
        ssh_key_path: guest::ensure_guest_key(paths)?
            .to_string_lossy()
            .into_owned(),
        api_socket: paths
            .machine_socket_path(&options.name)?
            .to_string_lossy()
            .into_owned(),
        config_path: paths
            .machine_config_path(&options.name)?
            .to_string_lossy()
            .into_owned(),
        serial_log_path: paths
            .machine_serial_path(&options.name)?
            .to_string_lossy()
            .into_owned(),
        process: None,
        created_at_unix_ms: now,
        updated_at_unix_ms: now,
        last_error: None,
        raw: BTreeMap::new(),
    };
    if let Err(error) = guest::create_seed(paths, &record, &seed_path) {
        let _ = fs::remove_dir_all(&machine_dir);
        return Err(error);
    }
    record.disks[1].logical_size_bytes = fs::metadata(&seed_path)?.len();
    record.disks[1].filesystem_uuid = filesystem_uuid(&seed_path).ok();
    save_machine(paths, &mut record)?;
    Ok(record)
}

pub fn start(paths: &RuntimePaths, name: &str, foreground: bool) -> Result<MachineRecord> {
    let _lock = MachineLock::acquire(paths, name)?;
    let mut record = load_machine(paths, name)?;
    reconcile_locked(paths, &mut record)?;
    if matches!(record.state, MachineState::Running | MachineState::Ready) {
        return Ok(record);
    }
    if record.state == MachineState::Stale {
        bail!(
            "machine {name} has ambiguous stale runtime state; inspect and resolve it before start"
        );
    }

    let manifest = load_manifest(paths)?;
    manifest.verify()?;
    verify_base_immutable(&record)?;
    network::create(name, &record.network)?;
    record.state = MachineState::Starting;
    record.updated_at_unix_ms = now_unix_ms();
    record.last_error = None;
    save_machine(paths, &mut record)?;

    match firecracker::launch(paths, &record, foreground) {
        Ok(identity) => {
            record.process = Some(identity);
            record.state = MachineState::Running;
            for disk in &mut record.disks {
                disk.attached = true;
            }
            record.updated_at_unix_ms = now_unix_ms();
            save_machine(paths, &mut record)?;
            if !foreground {
                if let Err(error) = guest::wait_for_ssh(&record, Duration::from_secs(120)) {
                    record.state = if record
                        .process
                        .as_ref()
                        .map(firecracker::verify_process)
                        .transpose()?
                        .unwrap_or(false)
                    {
                        MachineState::Running
                    } else {
                        MachineState::Crashed
                    };
                    record.last_error = Some(TypedError::new("GUEST_NOT_READY", error.to_string()));
                    record.updated_at_unix_ms = now_unix_ms();
                    save_machine(paths, &mut record)?;
                    return Err(error);
                }
                record.state = MachineState::Ready;
                record.updated_at_unix_ms = now_unix_ms();
                save_machine(paths, &mut record)?;
            }
            Ok(record)
        }
        Err(error) => {
            let _ = network::cleanup(name, &record.network);
            record.state = MachineState::Crashed;
            record.last_error = Some(TypedError::new(
                "FIRECRACKER_START_FAILED",
                error.to_string(),
            ));
            record.updated_at_unix_ms = now_unix_ms();
            save_machine(paths, &mut record)?;
            Err(error)
        }
    }
}

pub fn wait(paths: &RuntimePaths, name: &str, timeout: Duration) -> Result<MachineRecord> {
    let deadline = Instant::now() + timeout;
    let mut last = status(paths, name)?;
    while Instant::now() < deadline {
        if last.state == MachineState::Ready {
            return Ok(last);
        }
        if matches!(
            last.state,
            MachineState::Crashed | MachineState::Stale | MachineState::Stopped
        ) {
            bail!("machine {name} reached {:?} before ready", last.state);
        }
        thread::sleep(Duration::from_millis(500));
        last = status(paths, name)?;
    }
    bail!("timed out waiting for machine {name} to become ready")
}

pub fn status(paths: &RuntimePaths, name: &str) -> Result<MachineRecord> {
    let _lock = MachineLock::acquire(paths, name)?;
    let mut record = load_machine(paths, name)?;
    reconcile_locked(paths, &mut record)?;
    Ok(record)
}

pub fn reconcile(paths: &RuntimePaths, name: &str) -> Result<MachineRecord> {
    status(paths, name)
}

fn reconcile_locked(paths: &RuntimePaths, record: &mut MachineRecord) -> Result<()> {
    let prior = record.state.clone();
    match &record.process {
        Some(identity) if firecracker::verify_process(identity)? => {
            record.state = if guest::wait_for_ssh(record, Duration::from_millis(600)).is_ok() {
                MachineState::Ready
            } else {
                MachineState::Running
            };
        }
        Some(_) => {
            if Path::new(&record.api_socket).exists() || network::exists(&record.network) {
                record.state = MachineState::Stale;
            } else {
                record.process = None;
                record.state = if prior == MachineState::Starting {
                    MachineState::Crashed
                } else {
                    MachineState::Stopped
                };
                for disk in &mut record.disks {
                    disk.attached = false;
                }
            }
        }
        None => {
            if Path::new(&record.api_socket).exists() || network::exists(&record.network) {
                record.state = MachineState::Stale;
            } else if matches!(
                record.state,
                MachineState::Starting | MachineState::Running | MachineState::Ready
            ) {
                record.state = MachineState::Crashed;
            }
        }
    }
    if record.state != prior {
        record.updated_at_unix_ms = now_unix_ms();
        save_machine(paths, record)?;
    }
    Ok(())
}

pub fn stop(paths: &RuntimePaths, name: &str) -> Result<MachineRecord> {
    let _lock = MachineLock::acquire(paths, name)?;
    let mut record = load_machine(paths, name)?;
    reconcile_locked(paths, &mut record)?;
    let Some(identity) = record.process.clone() else {
        return Ok(record);
    };
    if record.state == MachineState::Stale {
        bail!("refusing graceful stop for ambiguous machine {name}");
    }
    let _ = guest::exec_capture(
        &record,
        &["systemctl".to_owned(), "poweroff".to_owned()],
        None,
    );
    if !firecracker::wait_for_exit(&identity, Duration::from_secs(30))? {
        let _ = firecracker::request_shutdown(&record);
    }
    if !firecracker::wait_for_exit(&identity, Duration::from_secs(10))? {
        firecracker::signal_verified(&identity, libc::SIGTERM)?;
    }
    if !firecracker::wait_for_exit(&identity, Duration::from_secs(10))? {
        bail!("machine {name} did not stop; use smp kill after inspecting its process identity");
    }
    finish_stopped(paths, &mut record)?;
    Ok(record)
}

pub fn kill(paths: &RuntimePaths, name: &str) -> Result<MachineRecord> {
    let _lock = MachineLock::acquire(paths, name)?;
    let mut record = load_machine(paths, name)?;
    let identity = record
        .process
        .clone()
        .ok_or_else(|| anyhow::anyhow!("machine {name} has no recorded process"))?;
    firecracker::signal_verified(&identity, libc::SIGKILL)?;
    if !firecracker::wait_for_exit(&identity, Duration::from_secs(10))? {
        bail!("verified Firecracker process did not exit after SIGKILL");
    }
    finish_stopped(paths, &mut record)?;
    Ok(record)
}

fn finish_stopped(paths: &RuntimePaths, record: &mut MachineRecord) -> Result<()> {
    let _ = network::cleanup(&record.name, &record.network);
    safe_remove_file(Path::new(&record.api_socket))?;
    record.process = None;
    record.state = MachineState::Stopped;
    for disk in &mut record.disks {
        disk.attached = false;
    }
    record.updated_at_unix_ms = now_unix_ms();
    save_machine(paths, record)
}

pub fn reboot(
    paths: &RuntimePaths,
    name: &str,
) -> Result<(crate::model::ProcessIdentity, MachineRecord)> {
    let before = status(paths, name)?;
    let old = before
        .process
        .clone()
        .ok_or_else(|| anyhow::anyhow!("machine {name} is not running"))?;
    let _ = guest::exec_capture(
        &before,
        &["systemctl".to_owned(), "reboot".to_owned()],
        None,
    );
    if !firecracker::wait_for_exit(&old, Duration::from_secs(30))? {
        let _ = stop(paths, name)?;
    } else {
        let _lock = MachineLock::acquire(paths, name)?;
        let mut record = load_machine(paths, name)?;
        finish_stopped(paths, &mut record)?;
    }
    let current = start(paths, name, false)?;
    let new = current
        .process
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("restarted machine has no process identity"))?;
    if old.pid == new.pid && old.start_time_ticks == new.start_time_ticks {
        bail!("reboot did not produce a new Firecracker process identity");
    }
    Ok((old, current))
}

pub fn destroy(paths: &RuntimePaths, name: &str, force: bool) -> Result<()> {
    let current = status(paths, name)?;
    if matches!(
        current.state,
        MachineState::Running | MachineState::Ready | MachineState::Starting
    ) {
        if !force {
            bail!("machine {name} is running; stop it or pass --force");
        }
        let _ = kill(paths, name)?;
    }
    let _lock = MachineLock::acquire(paths, name)?;
    let record = load_machine(paths, name)?;
    if record.state == MachineState::Stale {
        bail!("refusing to destroy ambiguous stale machine {name}");
    }
    network::cleanup(name, &record.network)?;
    safe_remove_file(Path::new(&record.api_socket))?;
    remove_machine_dir(paths, name)
}

pub fn ssh(paths: &RuntimePaths, name: &str) -> Result<i32> {
    let record = wait(paths, name, Duration::from_secs(120))?;
    Ok(guest::open_shell(&record)?.code().unwrap_or(255))
}

pub fn exec(paths: &RuntimePaths, name: &str, argv: &[String], tty: bool) -> Result<i32> {
    let record = wait(paths, name, Duration::from_secs(120))?;
    Ok(guest::exec_exact(&record, argv, tty)?.code().unwrap_or(255))
}

pub fn up(paths: &RuntimePaths, options: &CreateOptions) -> Result<i32> {
    if !paths.machine_state_path(&options.name)?.exists() {
        create(paths, options)?;
    }
    start(paths, &options.name, false)?;
    wait(paths, &options.name, Duration::from_secs(120))?;
    ssh(paths, &options.name)
}

pub fn copy(paths: &RuntimePaths, name: &str, source: &str, destination: &str) -> Result<()> {
    let record = wait(paths, name, Duration::from_secs(120))?;
    let source_guest = source.strip_prefix("guest:");
    let destination_guest = destination.strip_prefix("guest:");
    match (source_guest, destination_guest) {
        (Some(guest_path), None) => {
            guest::copy_guest_to_local(&record, guest_path, Path::new(destination))
        }
        (None, Some(guest_path)) => {
            guest::copy_local_to_guest(&record, Path::new(source), guest_path)
        }
        _ => bail!("exactly one cp endpoint must use the guest:/absolute/path form"),
    }
}

pub fn logs(paths: &RuntimePaths, name: &str, follow: bool, lines: u64) -> Result<i32> {
    let record = load_machine(paths, name)?;
    let mut command = Command::new("tail");
    command.arg("-n").arg(lines.to_string());
    if follow {
        command.arg("-F");
    }
    command.arg(&record.serial_log_path);
    let status = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    Ok(status.code().unwrap_or(1))
}

pub fn console(paths: &RuntimePaths, name: &str) -> Result<i32> {
    logs(paths, name, true, 200)
}

pub fn api(
    paths: &RuntimePaths,
    name: &str,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<(u16, Vec<u8>)> {
    let record = status(paths, name)?;
    firecracker::api_request(&record, method, path, headers, body)
}

pub fn verify_base_immutable(record: &MachineRecord) -> Result<()> {
    let observed = sha256_file(Path::new(&record.rootfs_base.path))?;
    if observed != record.rootfs_base.sha256 {
        bail!(
            "immutable base image digest changed: expected {}, observed {observed}",
            record.rootfs_base.sha256
        );
    }
    Ok(())
}

fn custom_or_manifest_identity(
    path: &Path,
    custom: bool,
    manifest: &AssetIdentity,
    custom_version: &str,
) -> Result<AssetIdentity> {
    if !path.is_file() {
        bail!("asset is missing: {}", path.display());
    }
    if custom {
        Ok(AssetIdentity {
            path: fs::canonicalize(path)?.to_string_lossy().into_owned(),
            sha256: sha256_file(path)?,
            version: custom_version.to_owned(),
            provenance_path: None,
        })
    } else {
        Ok(manifest.clone())
    }
}

fn filesystem_uuid(path: &Path) -> Result<String> {
    let output = run_output(
        "blkid",
        &[
            OsString::from("-s"),
            OsString::from("UUID"),
            OsString::from("-o"),
            OsString::from("value"),
            path.as_os_str().to_owned(),
        ],
    )?;
    if !output.status.success() {
        bail!(
            "blkid failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let value = String::from_utf8(output.stdout)?.trim().to_owned();
    if value.is_empty() {
        bail!("filesystem {} has no UUID", path.display());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_pci_boot_args_keep_interrupts_and_stable_network_name() {
        let args = default_boot_args("test");
        assert!(!args.split_whitespace().any(|value| value == "noapic"));
        assert!(args.split_whitespace().any(|value| value == "pci=on"));
        assert!(args
            .split_whitespace()
            .any(|value| value == "net.ifnames=0"));
    }
}
