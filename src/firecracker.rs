use crate::model::{MachineRecord, MachineState, ProcessIdentity, VirtioTransport};
use crate::state::{list_machines, RuntimePaths};
use crate::util::{atomic_write_json, open_append, process_matches, process_start_time_ticks};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub fn generate_config(record: &MachineRecord) -> Result<Value> {
    let root = record
        .disks
        .iter()
        .find(|disk| disk.drive_id == "root")
        .ok_or_else(|| anyhow::anyhow!("machine has no root disk"))?;
    let seed = record.disks.iter().find(|disk| disk.drive_id == "seed");
    let mut drives = vec![json!({
        "drive_id": "root",
        "path_on_host": root.path,
        "is_root_device": true,
        "is_read_only": false,
        "cache_type": "Unsafe",
        "io_engine": "Sync"
    })];
    if let Some(seed) = seed {
        drives.push(json!({
            "drive_id": "seed",
            "path_on_host": seed.path,
            "is_root_device": false,
            "is_read_only": true,
            "cache_type": "Unsafe",
            "io_engine": "Sync"
        }));
    }
    for disk in record
        .disks
        .iter()
        .filter(|disk| disk.drive_id != "root" && disk.drive_id != "seed")
    {
        drives.push(json!({
            "drive_id": disk.drive_id,
            "path_on_host": disk.path,
            "is_root_device": false,
            "is_read_only": !disk.writable,
            "cache_type": "Unsafe",
            "io_engine": "Sync"
        }));
    }

    let mut boot_args = record.boot_args.clone();
    match record.transport {
        VirtioTransport::Pci => {
            if !boot_args.split_whitespace().any(|value| value.starts_with("pci=")) {
                boot_args.push_str(" pci=on");
            }
        }
        VirtioTransport::Mmio => {
            boot_args = boot_args
                .split_whitespace()
                .filter(|value| !value.starts_with("pci="))
                .collect::<Vec<_>>()
                .join(" ");
            boot_args.push_str(" pci=off");
        }
    }

    let mut config = json!({
        "boot-source": {
            "kernel_image_path": record.kernel.path,
            "boot_args": boot_args
        },
        "drives": drives,
        "machine-config": {
            "vcpu_count": record.vcpu_count,
            "mem_size_mib": record.memory_mib,
            "smt": false,
            "track_dirty_pages": false
        },
        "network-interfaces": [{
            "iface_id": "eth0",
            "host_dev_name": record.network.tap_name,
            "guest_mac": record.network.guest_mac
        }],
        "entropy": {}
    });
    let object = config
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("generated Firecracker config is not an object"))?;
    for (key, value) in &record.raw {
        object.insert(key.clone(), value.clone());
    }
    Ok(config)
}

pub fn write_config(record: &MachineRecord) -> Result<()> {
    let config = generate_config(record)?;
    atomic_write_json(Path::new(&record.config_path), &config, 0o600)
}

pub fn reject_shared_writable_disk(
    paths: &RuntimePaths,
    candidate: &MachineRecord,
) -> Result<()> {
    for disk in candidate.disks.iter().filter(|disk| disk.writable) {
        let candidate_path = canonical_or_declared(&disk.path);
        for other in list_machines(paths)? {
            if other.name == candidate.name
                || !matches!(
                    other.state,
                    MachineState::Starting | MachineState::Running | MachineState::Ready
                )
            {
                continue;
            }
            let Some(process) = &other.process else {
                continue;
            };
            if !process_matches(
                process.pid,
                process.start_time_ticks,
                Path::new(&process.executable),
                &process.executable_sha256,
            )? {
                continue;
            }
            for other_disk in other.disks.iter().filter(|value| value.writable) {
                if canonical_or_declared(&other_disk.path) == candidate_path {
                    bail!(
                        "writable disk {} is already attached to running machine {}",
                        disk.path,
                        other.name
                    );
                }
            }
        }
    }
    Ok(())
}

pub fn launch(
    paths: &RuntimePaths,
    record: &MachineRecord,
    foreground: bool,
) -> Result<ProcessIdentity> {
    reject_shared_writable_disk(paths, record)?;
    write_config(record)?;
    let socket = Path::new(&record.api_socket);
    if let Some(parent) = socket.parent() {
        fs::create_dir_all(parent)?;
    }
    if socket.exists() {
        fs::remove_file(socket)
            .with_context(|| format!("remove stale socket {}", socket.display()))?;
    }

    let mut command = Command::new(&record.firecracker.path);
    command
        .arg("--api-sock")
        .arg(&record.api_socket)
        .arg("--config-file")
        .arg(&record.config_path);
    if record.transport == VirtioTransport::Pci {
        command.arg("--enable-pci");
    }

    if foreground {
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
    } else {
        let serial = open_append(Path::new(&record.serial_log_path), 0o600)?;
        let errors = serial.try_clone()?;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::from(serial))
            .stderr(Stdio::from(errors));
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    let child = command.spawn().context("launch Firecracker")?;
    let pid = child.id();
    let deadline = Instant::now() + Duration::from_secs(3);
    let start_time_ticks = loop {
        match process_start_time_ticks(pid) {
            Ok(value) => break value,
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Err(error) => return Err(error).context("identify Firecracker process"),
        }
    };
    let identity = ProcessIdentity {
        pid,
        start_time_ticks,
        executable: record.firecracker.path.clone(),
        executable_sha256: record.firecracker.sha256.clone(),
    };
    if !verify_process(&identity)? {
        bail!("new Firecracker process identity could not be verified");
    }
    Ok(identity)
}

pub fn verify_process(identity: &ProcessIdentity) -> Result<bool> {
    process_matches(
        identity.pid,
        identity.start_time_ticks,
        Path::new(&identity.executable),
        &identity.executable_sha256,
    )
}

pub fn signal_verified(identity: &ProcessIdentity, signal: i32) -> Result<()> {
    if !verify_process(identity)? {
        bail!("refusing to signal stale or mismatched Firecracker process");
    }
    let result = unsafe { libc::kill(identity.pid as i32, signal) };
    if result != 0 {
        return Err(std::io::Error::last_os_error()).context("signal Firecracker");
    }
    Ok(())
}

pub fn wait_for_exit(identity: &ProcessIdentity, timeout: Duration) -> Result<bool> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !verify_process(identity)? {
            return Ok(true);
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(!verify_process(identity)?)
}

pub fn api_request(
    record: &MachineRecord,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<(u16, Vec<u8>)> {
    if !path.starts_with('/')
        || path.contains("..")
        || path.contains('\n')
        || path.contains('\r')
    {
        bail!("invalid Firecracker API path");
    }
    if method.is_empty() || !method.bytes().all(|value| value.is_ascii_uppercase()) {
        bail!("invalid Firecracker API method");
    }
    let identity = record
        .process
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("machine has no process identity"))?;
    if !verify_process(identity)? {
        bail!("selected machine Firecracker process is not verified");
    }
    let expected_socket = Path::new(&record.api_socket);
    let is_socket = fs::metadata(expected_socket)
        .map(|metadata| metadata.file_type().is_socket())
        .unwrap_or(false);
    if !is_socket {
        bail!("selected machine API socket is unavailable or not a Unix socket");
    }
    let mut stream = UnixStream::connect(expected_socket)
        .with_context(|| format!("connect {}", expected_socket.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nAccept: application/json\r\nContent-Length: {}\r\n",
        body.len()
    )?;
    for (name, value) in headers {
        if name.contains('\r')
            || name.contains('\n')
            || value.contains('\r')
            || value.contains('\n')
        {
            bail!("invalid Firecracker API header");
        }
        write!(stream, "{name}: {value}\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    stream.write_all(body)?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    let separator = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("malformed Firecracker API response"))?;
    let head = String::from_utf8_lossy(&response[..separator]);
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| anyhow::anyhow!("malformed Firecracker API status"))?;
    Ok((status, response[separator + 4..].to_vec()))
}

pub fn request_shutdown(record: &MachineRecord) -> Result<()> {
    let (status, body) = api_request(
        record,
        "PUT",
        "/actions",
        &[("Content-Type".to_owned(), "application/json".to_owned())],
        br#"{"action_type":"SendCtrlAltDel"}"#,
    )?;
    if !(200..300).contains(&status) {
        bail!(
            "Firecracker shutdown request failed with HTTP {status}: {}",
            String::from_utf8_lossy(&body)
        );
    }
    Ok(())
}

fn canonical_or_declared(path: &str) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        AssetIdentity, DiskRecord, MachineMode, NetworkRecord, PublishedPort,
    };

    fn record() -> MachineRecord {
        MachineRecord {
            schema_version: 1,
            name: "default".to_owned(),
            architecture: "x86_64".to_owned(),
            mode: MachineMode::Persistent,
            state: MachineState::Created,
            transport: VirtioTransport::Pci,
            vcpu_count: 2,
            memory_mib: 1024,
            boot_args: "root=UUID=test rw console=ttyS0 reboot=k".to_owned(),
            firecracker: AssetIdentity {
                path: "/fc".to_owned(),
                sha256: "x".to_owned(),
                version: "1.15.1".to_owned(),
                provenance_path: None,
            },
            kernel: AssetIdentity {
                path: "/vmlinux".to_owned(),
                sha256: "y".to_owned(),
                version: "6.1.177".to_owned(),
                provenance_path: None,
            },
            rootfs_base: AssetIdentity {
                path: "/rootfs".to_owned(),
                sha256: "z".to_owned(),
                version: "13.6".to_owned(),
                provenance_path: None,
            },
            disks: vec![DiskRecord {
                drive_id: "root".to_owned(),
                path: "/machine/root.ext4".to_owned(),
                logical_size_bytes: 1,
                filesystem_uuid: Some("test".to_owned()),
                writable: true,
                attached: false,
                base_image_sha256: Some("z".to_owned()),
            }],
            network: NetworkRecord {
                tap_name: "smp0".to_owned(),
                guest_mac: "06:53:4d:00:00:01".to_owned(),
                guest_address: "172.31.4.2".to_owned(),
                gateway_address: "172.31.4.1".to_owned(),
                prefix_length: 30,
                dns_servers: vec![],
                published_ports: Vec::<PublishedPort>::new(),
                managed: true,
            },
            ssh_user: "root".to_owned(),
            ssh_key_path: "/key".to_owned(),
            api_socket: "/run/smp/default.firecracker.sock".to_owned(),
            config_path: "/var/lib/smp/machines/default/firecracker.json".to_owned(),
            serial_log_path: "/var/lib/smp/machines/default/serial.log".to_owned(),
            process: None,
            created_at_unix_ms: 0,
            updated_at_unix_ms: 0,
            last_error: None,
            raw: Default::default(),
        }
    }

    #[test]
    fn config_enables_entropy_and_preserves_raw_paths() {
        let config = generate_config(&record()).unwrap();
        assert!(config.get("entropy").is_some());
        assert_eq!(
            config["drives"][0]["path_on_host"],
            "/machine/root.ext4"
        );
    }

    #[test]
    fn mmio_forces_pci_off() {
        let mut machine = record();
        machine.transport = VirtioTransport::Mmio;
        let config = generate_config(&machine).unwrap();
        assert!(config["boot-source"]["boot_args"]
            .as_str()
            .unwrap()
            .contains("pci=off"));
    }
}
