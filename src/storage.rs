use crate::error::{Result, SmpError};
use crate::model::{DiskAttachment, MachineMode, MachineRecord, MachineState};
use crate::util::{command_checked, command_output, ensure_beneath, physical_size, sha256_file};
use std::fs::{self, OpenOptions};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub fn clone_base(base: &Path, destination: &Path, _mode: MachineMode) -> Result<DiskAttachment> {
    if destination.exists() {
        return Err(SmpError::Conflict(format!(
            "destination disk exists: {}",
            destination.display()
        )));
    }
    let base_metadata =
        fs::metadata(base).map_err(|error| SmpError::io(base.display().to_string(), error))?;
    if !base_metadata.is_file() {
        return Err(SmpError::Invalid(format!(
            "base image is not a regular file: {}",
            base.display()
        )));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| SmpError::io(parent.display().to_string(), error))?;
    }
    let args = vec![
        "--reflink=auto".to_owned(),
        "--sparse=always".to_owned(),
        "--".to_owned(),
        base.display().to_string(),
        destination.display().to_string(),
    ];
    command_checked("cp", &args)?;
    fs::set_permissions(destination, fs::Permissions::from_mode(0o600))
        .map_err(|error| SmpError::io(destination.display().to_string(), error))?;
    let logical_size = fs::metadata(destination)
        .map_err(|error| SmpError::io(destination.display().to_string(), error))?
        .len();
    let filesystem_uuid = filesystem_uuid(destination).ok();
    Ok(DiskAttachment {
        id: "root".to_owned(),
        path: destination.to_path_buf(),
        digest: None,
        filesystem_uuid,
        logical_size,
        physical_size: physical_size(destination)?,
        read_only: false,
        is_root: true,
        active: false,
    })
}

pub fn prepare_filesystem_for_uuid_change(path: &Path) -> Result<()> {
    let args = vec!["-p".to_owned(), "-f".to_owned(), path.display().to_string()];
    let output = command_output("e2fsck", &args)?;
    match output.status.code() {
        Some(0 | 1) => Ok(()),
        code => Err(SmpError::External {
            program: format!("e2fsck {}", args.join(" ")),
            code: code.unwrap_or(128),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        }),
    }
}

pub fn ensure_base_immutable(base: &Path, expected_digest: &str) -> Result<()> {
    let actual = sha256_file(base)?;
    if actual != expected_digest {
        return Err(SmpError::State(format!(
            "base image digest mismatch: expected {expected_digest}, got {actual}"
        )));
    }
    let metadata =
        fs::metadata(base).map_err(|error| SmpError::io(base.display().to_string(), error))?;
    if !metadata.permissions().readonly() {
        return Err(SmpError::State(format!(
            "canonical base image is writable: {}",
            base.display()
        )));
    }
    Ok(())
}

pub fn assert_writable_attachment_available(
    candidate: &Path,
    records: &[MachineRecord],
    selected_machine: &str,
) -> Result<()> {
    let candidate = canonical_or_lexical(candidate)?;
    for record in records {
        if record.machine_id == selected_machine {
            continue;
        }
        if matches!(
            record.state,
            MachineState::Starting | MachineState::Running | MachineState::Ready
        ) {
            for disk in std::iter::once(&record.root_disk).chain(&record.additional_disks) {
                if !disk.read_only && canonical_or_lexical(&disk.path)? == candidate {
                    return Err(SmpError::Conflict(format!(
                        "writable disk {} is active on {}",
                        candidate.display(),
                        record.machine_id
                    )));
                }
            }
        }
    }
    Ok(())
}

pub fn remove_declared_disk(machine_dir: &Path, disk: &DiskAttachment) -> Result<()> {
    ensure_beneath(machine_dir, &disk.path)?;
    if disk.active {
        return Err(SmpError::Conflict(format!(
            "disk {} is still active",
            disk.path.display()
        )));
    }
    match fs::remove_file(&disk.path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(SmpError::io(disk.path.display().to_string(), error)),
    }
}

pub fn resize_disk(path: &Path, new_size: u64) -> Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|error| SmpError::io(path.display().to_string(), error))?;
    let current = file
        .metadata()
        .map_err(|error| SmpError::io(path.display().to_string(), error))?
        .len();
    if new_size <= current {
        return Err(SmpError::Invalid(format!(
            "new disk size {new_size} must exceed {current}"
        )));
    }
    file.set_len(new_size)
        .map_err(|error| SmpError::io(path.display().to_string(), error))
}

pub fn filesystem_uuid(path: &Path) -> Result<String> {
    let output = command_checked(
        "blkid",
        &[
            "-s".to_owned(),
            "UUID".to_owned(),
            "-o".to_owned(),
            "value".to_owned(),
            path.display().to_string(),
        ],
    )?;
    let uuid = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if uuid.is_empty() {
        Err(SmpError::State(format!(
            "filesystem UUID unavailable: {}",
            path.display()
        )))
    } else {
        Ok(uuid)
    }
}

fn canonical_or_lexical(path: &Path) -> Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(value) => Ok(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && path.is_absolute() => {
            Ok(path.to_path_buf())
        }
        Err(error) => Err(SmpError::io(path.display().to_string(), error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NetworkDefinition, ProcessIdentity, Transport};

    fn record(path: PathBuf, state: MachineState, name: &str) -> MachineRecord {
        MachineRecord {
            schema_version: 1,
            machine_id: name.to_owned(),
            mode: MachineMode::Persistent,
            architecture: "x86_64".to_owned(),
            transport: Transport::Pci,
            vcpu_count: 1,
            memory_mib: 128,
            firecracker_path: "/fc".into(),
            firecracker_digest: "f".to_owned(),
            kernel_path: "/kernel".into(),
            kernel_digest: "k".to_owned(),
            kernel_arguments: String::new(),
            initrd_path: None,
            root_disk: DiskAttachment {
                id: "root".to_owned(),
                path,
                digest: None,
                filesystem_uuid: None,
                logical_size: 1,
                physical_size: 1,
                read_only: false,
                is_root: true,
                active: true,
            },
            additional_disks: vec![],
            base_image_digest: "b".to_owned(),
            network: NetworkDefinition {
                tap: "smp1".to_owned(),
                subnet: "172.31.1.0".to_owned(),
                prefix_length: 30,
                guest_address: "172.31.1.2".to_owned(),
                gateway: "172.31.1.1".to_owned(),
                dns: vec![],
                guest_mac: "06:00:00:00:00:01".to_owned(),
                published_ports: vec![],
            },
            seed_path: "/seed".into(),
            seed_identity: "s".to_owned(),
            machine_directory: "/machine".into(),
            api_socket: "/machine/api.sock".into(),
            firecracker_process: Some(ProcessIdentity {
                pid: 1,
                process_start_time: 1,
                executable_path: "/fc".into(),
                executable_digest: None,
                boot_id: "b".to_owned(),
                process_group: 1,
            }),
            generated_config_digest: None,
            created_at: 1,
            updated_at: 1,
            state,
            last_error: None,
        }
    }

    #[test]
    fn readonly_base_clone_is_private_writable_and_uuid_ready() -> Result<()> {
        let directory = tempfile::tempdir().map_err(|error| SmpError::io("tempdir", error))?;
        let base = directory.path().join("base.ext4");
        let clone = directory.path().join("clone.ext4");
        command_checked(
            "truncate",
            &[
                "-s".to_owned(),
                "32M".to_owned(),
                base.display().to_string(),
            ],
        )?;
        command_checked(
            "mkfs.ext4",
            &["-q".to_owned(), "-F".to_owned(), base.display().to_string()],
        )?;
        let base_uuid = filesystem_uuid(&base)?;
        fs::set_permissions(&base, fs::Permissions::from_mode(0o444))
            .map_err(|error| SmpError::io(base.display().to_string(), error))?;

        let attachment = clone_base(&base, &clone, MachineMode::Persistent)?;
        assert_eq!(attachment.path, clone);
        assert_eq!(
            fs::metadata(&base)
                .map_err(|error| SmpError::io("base", error))?
                .permissions()
                .mode()
                & 0o777,
            0o444
        );
        assert_eq!(
            fs::metadata(&clone)
                .map_err(|error| SmpError::io("clone", error))?
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        prepare_filesystem_for_uuid_change(&clone)?;
        command_checked(
            "tune2fs",
            &[
                "-U".to_owned(),
                "random".to_owned(),
                clone.display().to_string(),
            ],
        )?;
        assert_ne!(filesystem_uuid(&clone)?, base_uuid);
        assert_eq!(filesystem_uuid(&base)?, base_uuid);
        Ok(())
    }

    #[test]
    fn writable_attachment_conflict_is_detected() {
        let records = vec![record(
            PathBuf::from("/var/lib/smp/machines/one/root.ext4"),
            MachineState::Ready,
            "one",
        )];
        assert!(
            assert_writable_attachment_available(
                Path::new("/var/lib/smp/machines/one/root.ext4"),
                &records,
                "two"
            )
            .is_err()
        );
    }

    #[test]
    fn stopped_disk_does_not_conflict() {
        let records = vec![record(
            PathBuf::from("/var/lib/smp/machines/one/root.ext4"),
            MachineState::Stopped,
            "one",
        )];
        assert!(
            assert_writable_attachment_available(
                Path::new("/var/lib/smp/machines/one/root.ext4"),
                &records,
                "two"
            )
            .is_ok()
        );
    }
}
