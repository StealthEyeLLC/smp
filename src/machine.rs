use crate::assets;
use crate::error::{Result, SmpError};
use crate::firecracker;
use crate::guest;
use crate::model::{
    MachineMode, MachineRecord, MachineState, NetworkDefinition, PortPublication, ProcessIdentity,
    Transport,
};
use crate::network;
use crate::paths::{Paths, validate_machine_name};
use crate::process;
use crate::storage;
use crate::util::{
    atomic_json, atomic_write, command_checked, ensure_beneath, now_unix_seconds, read_json,
    sha256_file,
};
use fs2::FileExt;
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

const DEFAULT_BOOT_ARGUMENTS: &str =
    "console=ttyS0 reboot=k panic=1 pci=on rw init=/sbin/init random.trust_cpu=on";

#[derive(Clone, Debug)]
pub struct Manager {
    pub paths: Paths,
}

#[derive(Clone, Debug)]
pub struct CreateOptions {
    pub machine_id: String,
    pub mode: MachineMode,
    pub transport: Transport,
    pub vcpu_count: u8,
    pub memory_mib: u32,
    pub firecracker: Option<PathBuf>,
    pub kernel: Option<PathBuf>,
    pub rootfs: Option<PathBuf>,
    pub initrd: Option<PathBuf>,
    pub kernel_arguments: Option<String>,
    pub published_ports: Vec<PortPublication>,
    pub initialization_script: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RebootResult {
    pub old_process: ProcessIdentity,
    pub new_process: ProcessIdentity,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogChunk {
    pub path: String,
    pub offset: u64,
    pub next_offset: u64,
    pub data: Vec<u8>,
    pub eof: bool,
    pub truncated: bool,
}

impl Manager {
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }

    pub fn create(&self, options: CreateOptions) -> Result<MachineRecord> {
        self.paths.ensure_layout()?;
        validate_machine_name(&options.machine_id)?;
        if options.vcpu_count == 0 || options.memory_mib < 64 {
            return Err(SmpError::Invalid(
                "vCPU count must be positive and memory at least 64 MiB".to_owned(),
            ));
        }
        let _lock = self.lock(&options.machine_id)?;
        let machine_dir = self.paths.machine_dir(&options.machine_id)?;
        let record_path = self.paths.machine_record(&options.machine_id)?;
        if record_path.exists() {
            return Err(SmpError::Conflict(format!(
                "machine {} already exists",
                options.machine_id
            )));
        }
        if machine_dir.exists()
            && fs::read_dir(&machine_dir)
                .map_err(|error| SmpError::io(machine_dir.display().to_string(), error))?
                .next()
                .is_some()
        {
            return Err(SmpError::Conflict(format!(
                "machine directory is not empty: {}",
                machine_dir.display()
            )));
        }
        fs::create_dir_all(&machine_dir)
            .map_err(|error| SmpError::io(machine_dir.display().to_string(), error))?;
        fs::set_permissions(&machine_dir, fs::Permissions::from_mode(0o700))
            .map_err(|error| SmpError::io(machine_dir.display().to_string(), error))?;

        let verified = assets::verify(&self.paths)?;
        let firecracker_path = options
            .firecracker
            .clone()
            .unwrap_or_else(|| verified.firecracker.clone());
        let kernel_path = options
            .kernel
            .clone()
            .unwrap_or_else(|| verified.kernel.clone());
        let base_rootfs = options
            .rootfs
            .clone()
            .unwrap_or_else(|| verified.rootfs.clone());
        for path in [&firecracker_path, &kernel_path, &base_rootfs] {
            if !path.is_absolute() || !path.is_file() {
                return Err(SmpError::Invalid(format!(
                    "component is not an absolute regular file: {}",
                    path.display()
                )));
            }
        }
        let firecracker_digest = sha256_file(&firecracker_path)?;
        let kernel_digest = sha256_file(&kernel_path)?;
        let base_image_digest = sha256_file(&base_rootfs)?;
        if options.rootfs.is_none() {
            storage::ensure_base_immutable(&base_rootfs, &verified.manifest.debian.rootfs_sha256)?;
        }

        let root_path = machine_dir.join("root.ext4");
        let result = (|| -> Result<MachineRecord> {
            let mut root_disk =
                storage::clone_base(&base_rootfs, &root_path, options.mode.clone())?;
            command_checked(
                "tune2fs",
                &[
                    "-U".to_owned(),
                    "random".to_owned(),
                    root_path.display().to_string(),
                ],
            )?;
            root_disk.filesystem_uuid = Some(storage::filesystem_uuid(&root_path)?);
            let existing = self
                .list()?
                .into_iter()
                .map(|record| record.network)
                .collect::<Vec<_>>();
            let network = network::deterministic_definition(
                &options.machine_id,
                &existing,
                options.published_ports.clone(),
            )?;
            let seed_path = machine_dir.join("seed.ext4");
            let seed_identity = self.create_seed(
                &options.machine_id,
                &network,
                &seed_path,
                options.initialization_script.as_deref(),
            )?;
            let root_uuid = root_disk
                .filesystem_uuid
                .as_ref()
                .ok_or_else(|| SmpError::State("root filesystem UUID is missing".to_owned()))?;
            let kernel_arguments = options
                .kernel_arguments
                .unwrap_or_else(|| format!("{DEFAULT_BOOT_ARGUMENTS} root=UUID={root_uuid}"));
            let now = now_unix_seconds();
            let record = MachineRecord {
                schema_version: crate::MACHINE_SCHEMA_VERSION,
                machine_id: options.machine_id.clone(),
                mode: options.mode,
                architecture: "x86_64".to_owned(),
                transport: options.transport,
                vcpu_count: options.vcpu_count,
                memory_mib: options.memory_mib,
                firecracker_path,
                firecracker_digest,
                kernel_path,
                kernel_digest,
                kernel_arguments,
                initrd_path: options.initrd,
                root_disk,
                additional_disks: Vec::new(),
                base_image_digest,
                network,
                seed_path,
                seed_identity,
                machine_directory: machine_dir.clone(),
                api_socket: self.paths.machine_socket(&options.machine_id)?,
                firecracker_process: None,
                generated_config_digest: None,
                created_at: now,
                updated_at: now,
                state: MachineState::Created,
                last_error: None,
            };
            record.validate()?;
            self.save(&record)?;
            Ok(record)
        })();
        if result.is_err() {
            let _ = safe_remove_machine_dir(&self.paths.machines, &machine_dir);
        }
        result
    }

    pub fn start(&self, machine: &str, ready_timeout: Duration) -> Result<MachineRecord> {
        let _lock = self.lock(machine)?;
        let mut record = self.load(machine)?;
        if let Some(identity) = &record.firecracker_process
            && process::is_running(identity)
        {
            if record.state == MachineState::Ready {
                return Ok(record);
            }
            return Err(SmpError::Conflict(format!(
                "machine {machine} already has a live Firecracker process"
            )));
        }
        if record.api_socket.exists() {
            firecracker::remove_stale_socket(&record)?;
        }
        let records = self.list()?;
        storage::assert_writable_attachment_available(&record.root_disk.path, &records, machine)?;
        network::apply(machine, &record.network)?;
        record.state = MachineState::Starting;
        record.updated_at = now_unix_seconds();
        record.last_error = None;
        self.save(&record)?;
        match firecracker::launch(&record, &self.paths.runtime) {
            Ok(identity) => {
                record.firecracker_process = Some(identity);
                record.root_disk.active = true;
                for disk in &mut record.additional_disks {
                    disk.active = true;
                }
                record.generated_config_digest = Some(firecracker::configuration_digest(&record)?);
                record.state = MachineState::Running;
                record.updated_at = now_unix_seconds();
                self.save(&record)?;
            }
            Err(error) => {
                let _ = network::cleanup(machine, &record.network);
                record.state = MachineState::Crashed;
                record.last_error = Some(error.to_string());
                record.updated_at = now_unix_seconds();
                self.save(&record)?;
                return Err(error);
            }
        }
        match guest::ready(&record, &self.ssh_key(), ready_timeout) {
            Ok(()) => {
                record.state = MachineState::Ready;
                record.last_error = None;
                record.updated_at = now_unix_seconds();
                self.save(&record)?;
                Ok(record)
            }
            Err(error) => {
                if record
                    .firecracker_process
                    .as_ref()
                    .is_some_and(process::is_running)
                {
                    record.state = MachineState::Running;
                } else {
                    record.state = MachineState::Crashed;
                    record.root_disk.active = false;
                }
                record.last_error = Some(error.to_string());
                record.updated_at = now_unix_seconds();
                self.save(&record)?;
                Err(error)
            }
        }
    }

    pub fn wait(&self, machine: &str, timeout: Duration) -> Result<MachineRecord> {
        let _lock = self.lock(machine)?;
        let mut record = self.load(machine)?;
        let identity = record
            .firecracker_process
            .as_ref()
            .ok_or_else(|| SmpError::State("machine has no Firecracker process".to_owned()))?;
        process::verify(identity)?;
        guest::ready(&record, &self.ssh_key(), timeout)?;
        record.state = MachineState::Ready;
        record.last_error = None;
        record.updated_at = now_unix_seconds();
        self.save(&record)?;
        Ok(record)
    }

    pub fn stop(&self, machine: &str, timeout: Duration) -> Result<MachineRecord> {
        let _lock = self.lock(machine)?;
        let mut record = self.load(machine)?;
        let Some(identity) = record.firecracker_process.clone() else {
            if matches!(record.state, MachineState::Created | MachineState::Stopped) {
                return Ok(record);
            }
            record.state = MachineState::Stale;
            record.updated_at = now_unix_seconds();
            self.save(&record)?;
            return Err(SmpError::State(
                "machine state claims activity without a process identity".to_owned(),
            ));
        };
        process::verify(&identity)?;
        let _ = guest::execute(
            &record,
            &self.ssh_key(),
            &["systemctl".to_owned(), "poweroff".to_owned()],
            None,
            Duration::from_secs(10),
            128 * 1024,
            false,
        );
        if !process::wait_for_exit(&identity, timeout)? {
            process::signal(&identity, libc::SIGTERM)?;
            if !process::wait_for_exit(&identity, Duration::from_secs(10))? {
                process::signal(&identity, libc::SIGKILL)?;
                if !process::wait_for_exit(&identity, Duration::from_secs(10))? {
                    return Err(SmpError::State(
                        "verified Firecracker process did not exit after SIGKILL".to_owned(),
                    ));
                }
            }
        }
        self.mark_stopped(&mut record)?;
        Ok(record)
    }

    pub fn kill(&self, machine: &str) -> Result<MachineRecord> {
        let _lock = self.lock(machine)?;
        let mut record = self.load(machine)?;
        let identity = record
            .firecracker_process
            .clone()
            .ok_or_else(|| SmpError::State("machine has no Firecracker process".to_owned()))?;
        process::signal(&identity, libc::SIGKILL)?;
        if !process::wait_for_exit(&identity, Duration::from_secs(10))? {
            return Err(SmpError::State(
                "verified Firecracker process did not exit".to_owned(),
            ));
        }
        self.mark_stopped(&mut record)?;
        Ok(record)
    }

    pub fn reboot(&self, machine: &str, timeout: Duration) -> Result<RebootResult> {
        let before = self.load(machine)?;
        let old_process = before
            .firecracker_process
            .clone()
            .ok_or_else(|| SmpError::State("machine has no Firecracker process".to_owned()))?;
        self.stop(machine, timeout)?;
        let after = self.start(machine, timeout)?;
        let new_process = after
            .firecracker_process
            .clone()
            .ok_or_else(|| SmpError::State("restarted machine has no process".to_owned()))?;
        if old_process.pid == new_process.pid
            && old_process.process_start_time == new_process.process_start_time
        {
            return Err(SmpError::State(
                "host-mediated reboot did not replace process identity".to_owned(),
            ));
        }
        Ok(RebootResult {
            old_process,
            new_process,
        })
    }

    pub fn destroy(&self, machine: &str, delete_persistent_disk: bool) -> Result<()> {
        let record = self.load(machine)?;
        if record
            .firecracker_process
            .as_ref()
            .is_some_and(process::is_running)
        {
            return Err(SmpError::Conflict(
                "stop or kill the machine before destroy".to_owned(),
            ));
        }
        if record.mode == MachineMode::Persistent && !delete_persistent_disk {
            return Err(SmpError::Conflict(
                "persistent destroy requires --delete-disk".to_owned(),
            ));
        }
        let _lock = self.lock(machine)?;
        network::cleanup(machine, &record.network)?;
        if record.mode == MachineMode::Disposable || delete_persistent_disk {
            storage::remove_declared_disk(&record.machine_directory, &record.root_disk)?;
            for disk in &record.additional_disks {
                if !disk.read_only {
                    storage::remove_declared_disk(&record.machine_directory, disk)?;
                }
            }
        }
        safe_remove_machine_dir(&self.paths.machines, &record.machine_directory)
    }

    pub fn reconcile(&self, machine: &str) -> Result<MachineRecord> {
        let _lock = self.lock(machine)?;
        let mut record = self.load(machine)?;
        match record.firecracker_process.clone() {
            Some(identity) if process::verify(&identity).is_ok() => {
                network::apply(machine, &record.network)?;
                record.root_disk.active = true;
                record.state =
                    if guest::ready(&record, &self.ssh_key(), Duration::from_secs(2)).is_ok() {
                        MachineState::Ready
                    } else {
                        MachineState::Running
                    };
                record.last_error = None;
            }
            Some(identity) if Path::new(&format!("/proc/{}", identity.pid)).exists() => {
                return Err(SmpError::Ambiguous(format!(
                    "PID {} exists but does not match recorded Firecracker identity",
                    identity.pid
                )));
            }
            Some(_) => {
                firecracker::remove_stale_socket(&record)?;
                network::cleanup(machine, &record.network)?;
                record.firecracker_process = None;
                record.root_disk.active = false;
                for disk in &mut record.additional_disks {
                    disk.active = false;
                }
                record.state = MachineState::Crashed;
                record.last_error = Some("recorded Firecracker process is gone".to_owned());
            }
            None if matches!(
                record.state,
                MachineState::Starting | MachineState::Running | MachineState::Ready
            ) =>
            {
                record.state = MachineState::Stale;
                record.last_error =
                    Some("active state lacks a Firecracker process identity".to_owned());
            }
            None => {}
        }
        record.updated_at = now_unix_seconds();
        self.save(&record)?;
        Ok(record)
    }

    pub fn load(&self, machine: &str) -> Result<MachineRecord> {
        let path = self.paths.machine_record(machine)?;
        let record: MachineRecord = read_json(&path)?;
        record.validate()?;
        let expected = self.paths.machine_dir(machine)?;
        let expected_socket = self.paths.machine_socket(machine)?;
        if record.machine_directory != expected
            || record.api_socket != expected_socket
            || record.machine_id != machine
        {
            return Err(SmpError::Ambiguous(format!(
                "machine record path identity mismatch for {machine}"
            )));
        }
        Ok(record)
    }

    pub fn list(&self) -> Result<Vec<MachineRecord>> {
        if !self.paths.machines.exists() {
            return Ok(Vec::new());
        }
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.paths.machines)
            .map_err(|error| SmpError::io(self.paths.machines.display().to_string(), error))?
        {
            let entry = entry
                .map_err(|error| SmpError::io(self.paths.machines.display().to_string(), error))?;
            if !entry
                .file_type()
                .map_err(|error| SmpError::io(entry.path().display().to_string(), error))?
                .is_dir()
            {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if validate_machine_name(&name).is_err() {
                continue;
            }
            let record_path = entry.path().join("machine.json");
            if record_path.exists() {
                records.push(self.load(&name)?);
            }
        }
        records.sort_by(|left, right| left.machine_id.cmp(&right.machine_id));
        Ok(records)
    }

    pub fn read_log(
        &self,
        machine: &str,
        stream: &str,
        offset: u64,
        limit: usize,
    ) -> Result<LogChunk> {
        if limit == 0 || limit > 1024 * 1024 {
            return Err(SmpError::Invalid(
                "log limit must be 1 through 1048576".to_owned(),
            ));
        }
        let record = self.load(machine)?;
        let path = match stream {
            "stdout" | "serial" => record.machine_directory.join("firecracker.stdout.log"),
            "stderr" => record.machine_directory.join("firecracker.stderr.log"),
            _ => return Err(SmpError::Invalid(format!("unknown log stream {stream}"))),
        };
        let mut file =
            File::open(&path).map_err(|error| SmpError::io(path.display().to_string(), error))?;
        let size = file
            .metadata()
            .map_err(|error| SmpError::io(path.display().to_string(), error))?
            .len();
        let effective_offset = offset.min(size);
        file.seek(SeekFrom::Start(effective_offset))
            .map_err(|error| SmpError::io(path.display().to_string(), error))?;
        let mut data = vec![0_u8; limit];
        let count = file
            .read(&mut data)
            .map_err(|error| SmpError::io(path.display().to_string(), error))?;
        data.truncate(count);
        let next_offset = effective_offset.saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
        Ok(LogChunk {
            path: path.display().to_string(),
            offset: effective_offset,
            next_offset,
            data,
            eof: next_offset >= size,
            truncated: offset > size,
        })
    }

    pub fn ssh_key(&self) -> PathBuf {
        self.paths.credentials.join("id_ed25519")
    }

    fn save(&self, record: &MachineRecord) -> Result<()> {
        record.validate()?;
        atomic_json(
            &self.paths.machine_record(&record.machine_id)?,
            record,
            0o600,
        )
    }

    fn lock(&self, machine: &str) -> Result<File> {
        validate_machine_name(machine)?;
        fs::create_dir_all(&self.paths.runtime)
            .map_err(|error| SmpError::io(self.paths.runtime.display().to_string(), error))?;
        let path = self.paths.runtime.join(format!("{machine}.lock"));
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|error| SmpError::io(path.display().to_string(), error))?;
        file.lock_exclusive()
            .map_err(|error| SmpError::io(path.display().to_string(), error))?;
        Ok(file)
    }

    fn create_seed(
        &self,
        machine: &str,
        network: &NetworkDefinition,
        destination: &Path,
        initialization_script: Option<&Path>,
    ) -> Result<String> {
        let public_key = self.ensure_public_key()?;
        let parent = destination
            .parent()
            .ok_or_else(|| SmpError::Invalid("seed path has no parent".to_owned()))?;
        let seed_root = parent.join(format!(".seed-{}", Uuid::new_v4()));
        fs::create_dir_all(&seed_root)
            .map_err(|error| SmpError::io(seed_root.display().to_string(), error))?;
        let result = (|| -> Result<String> {
            atomic_write(
                &seed_root.join("manifest.json"),
                b"{\"schemaVersion\":1}\n",
                0o600,
            )?;
            atomic_write(
                &seed_root.join("hostname"),
                format!("{machine}\n").as_bytes(),
                0o600,
            )?;
            atomic_write(
                &seed_root.join("authorized_keys"),
                public_key.as_bytes(),
                0o600,
            )?;
            atomic_json(
                &seed_root.join("network.json"),
                &serde_json::json!({
                    "schemaVersion": 1,
                    "mac": network.guest_mac,
                    "address": network.guest_address,
                    "prefixLength": network.prefix_length,
                    "gateway": network.gateway,
                    "dns": network.dns
                }),
                0o600,
            )?;
            if let Some(script) = initialization_script {
                let bytes = fs::read(script)
                    .map_err(|error| SmpError::io(script.display().to_string(), error))?;
                atomic_write(&seed_root.join("init.sh"), &bytes, 0o700)?;
            }
            command_checked(
                "truncate",
                &[
                    "--size".to_owned(),
                    "16M".to_owned(),
                    destination.display().to_string(),
                ],
            )?;
            command_checked(
                "mkfs.ext4",
                &[
                    "-F".to_owned(),
                    "-L".to_owned(),
                    "SMP_SEED".to_owned(),
                    "-U".to_owned(),
                    "random".to_owned(),
                    "-d".to_owned(),
                    seed_root.display().to_string(),
                    destination.display().to_string(),
                ],
            )?;
            sha256_file(destination)
        })();
        let cleanup = fs::remove_dir_all(&seed_root);
        if let Err(error) = cleanup
            && result.is_ok()
        {
            return Err(SmpError::io(seed_root.display().to_string(), error));
        }
        result
    }

    fn ensure_public_key(&self) -> Result<String> {
        let private = self.ssh_key();
        let public = private.with_extension("pub");
        if public.exists() {
            let value = fs::read_to_string(&public)
                .map_err(|error| SmpError::io(public.display().to_string(), error))?;
            if value.trim().is_empty() {
                return Err(SmpError::State("SMP SSH public key is empty".to_owned()));
            }
            return Ok(format!("{}\n", value.trim()));
        }
        if !private.exists() {
            return Err(SmpError::NotFound(format!(
                "SMP SSH key is missing: {}",
                private.display()
            )));
        }
        let output = command_checked(
            "ssh-keygen",
            &[
                "-y".to_owned(),
                "-f".to_owned(),
                private.display().to_string(),
            ],
        )?;
        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if value.is_empty() {
            return Err(SmpError::State(
                "ssh-keygen returned an empty public key".to_owned(),
            ));
        }
        atomic_write(&public, format!("{value}\n").as_bytes(), 0o600)?;
        Ok(format!("{value}\n"))
    }

    fn mark_stopped(&self, record: &mut MachineRecord) -> Result<()> {
        network::cleanup(&record.machine_id, &record.network)?;
        firecracker::remove_stale_socket(record)?;
        record.firecracker_process = None;
        record.root_disk.active = false;
        for disk in &mut record.additional_disks {
            disk.active = false;
        }
        record.state = MachineState::Stopped;
        record.last_error = None;
        record.updated_at = now_unix_seconds();
        self.save(record)
    }
}

fn safe_remove_machine_dir(root: &Path, machine_dir: &Path) -> Result<()> {
    ensure_beneath(root, machine_dir)?;
    if machine_dir == root {
        return Err(SmpError::Invalid(
            "refusing to remove machine root".to_owned(),
        ));
    }
    match fs::remove_dir_all(machine_dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(SmpError::io(machine_dir.display().to_string(), error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_destroy_requires_explicit_disk_intent() {
        assert!(DEFAULT_BOOT_ARGUMENTS.contains("reboot=k"));
    }

    #[test]
    fn log_limits_are_bounded() -> Result<()> {
        let directory = tempfile::tempdir().map_err(|error| SmpError::io("tempdir", error))?;
        let paths = test_paths(directory.path());
        let manager = Manager::new(paths);
        assert!(manager.read_log("default", "stdout", 0, 0).is_err());
        assert!(
            manager
                .read_log("default", "stdout", 0, 1024 * 1024 + 1)
                .is_err()
        );
        Ok(())
    }

    fn test_paths(root: &Path) -> Paths {
        Paths {
            binary: root.join("bin/smp"),
            lib: root.join("lib"),
            config: root.join("etc"),
            credentials: root.join("etc/credentials"),
            state: root.join("state"),
            assets: root.join("state/assets"),
            machines: root.join("state/machines"),
            requests: root.join("state/requests"),
            results: root.join("state/results"),
            provenance: root.join("state/provenance"),
            runtime: root.join("run"),
        }
    }
}
