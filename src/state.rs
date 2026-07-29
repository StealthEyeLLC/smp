use crate::model::{MachineRecord, MachineState, RequestRecord, ResultRecord, MACHINE_SCHEMA_VERSION};
use crate::util::{atomic_write_json, read_json, validate_machine_name};
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct RuntimePaths {
    pub etc_root: PathBuf,
    pub state_root: PathBuf,
    pub run_root: PathBuf,
    pub lib_root: PathBuf,
}

impl Default for RuntimePaths {
    fn default() -> Self {
        Self {
            etc_root: std::env::var_os("SMP_ETC_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/etc/smp")),
            state_root: std::env::var_os("SMP_STATE_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/var/lib/smp")),
            run_root: std::env::var_os("SMP_RUN_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/run/smp")),
            lib_root: std::env::var_os("SMP_LIB_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/usr/lib/smp")),
        }
    }
}

impl RuntimePaths {
    pub fn ensure(&self) -> Result<()> {
        for path in [
            self.state_root.clone(),
            self.machines_root(),
            self.assets_root(),
            self.requests_root(),
            self.results_root(),
            self.run_root.clone(),
            self.locks_root(),
        ] {
            fs::create_dir_all(&path).with_context(|| format!("create {}", path.display()))?;
        }
        Ok(())
    }

    pub fn machines_root(&self) -> PathBuf {
        self.state_root.join("machines")
    }

    pub fn machine_dir(&self, name: &str) -> Result<PathBuf> {
        validate_machine_name(name)?;
        Ok(self.machines_root().join(name))
    }

    pub fn machine_state_path(&self, name: &str) -> Result<PathBuf> {
        Ok(self.machine_dir(name)?.join("machine.json"))
    }

    pub fn machine_config_path(&self, name: &str) -> Result<PathBuf> {
        Ok(self.machine_dir(name)?.join("firecracker.json"))
    }

    pub fn machine_socket_path(&self, name: &str) -> Result<PathBuf> {
        validate_machine_name(name)?;
        Ok(self.run_root.join(format!("{name}.firecracker.sock")))
    }

    pub fn machine_serial_path(&self, name: &str) -> Result<PathBuf> {
        Ok(self.machine_dir(name)?.join("serial.log"))
    }

    pub fn assets_root(&self) -> PathBuf {
        self.state_root.join("assets")
    }

    pub fn requests_root(&self) -> PathBuf {
        self.state_root.join("requests")
    }

    pub fn results_root(&self) -> PathBuf {
        self.state_root.join("results")
    }

    pub fn locks_root(&self) -> PathBuf {
        self.run_root.join("locks")
    }

    pub fn guest_key_path(&self) -> PathBuf {
        self.etc_root.join("credentials/guest_ed25519")
    }

    pub fn request_path(&self, request_id: &str) -> Result<PathBuf> {
        validate_record_id(request_id)?;
        Ok(self.requests_root().join(format!("{request_id}.json")))
    }

    pub fn result_dir(&self, handle: &str) -> Result<PathBuf> {
        validate_record_id(handle)?;
        Ok(self.results_root().join(handle))
    }
}

pub struct MachineLock {
    _file: File,
}

impl MachineLock {
    pub fn acquire(paths: &RuntimePaths, name: &str) -> Result<Self> {
        validate_machine_name(name)?;
        paths.ensure()?;
        let path = paths.locks_root().join(format!("{name}.lock"));
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&path)?;
        file.try_lock_exclusive()
            .with_context(|| format!("machine {name} is locked by another operation"))?;
        Ok(Self { _file: file })
    }
}

pub fn load_machine(paths: &RuntimePaths, name: &str) -> Result<MachineRecord> {
    let path = paths.machine_state_path(name)?;
    let record: MachineRecord = read_json(&path)?;
    if record.schema_version != MACHINE_SCHEMA_VERSION {
        bail!(
            "unsupported machine schema {} in {}",
            record.schema_version,
            path.display()
        );
    }
    if record.name != name {
        bail!("machine record name mismatch");
    }
    Ok(record)
}

pub fn save_machine(paths: &RuntimePaths, record: &mut MachineRecord) -> Result<()> {
    validate_machine_name(&record.name)?;
    record.schema_version = MACHINE_SCHEMA_VERSION;
    fs::create_dir_all(paths.machine_dir(&record.name)?)?;
    atomic_write_json(&paths.machine_state_path(&record.name)?, record, 0o600)
}

pub fn list_machines(paths: &RuntimePaths) -> Result<Vec<MachineRecord>> {
    paths.ensure()?;
    let mut records = Vec::new();
    for entry in fs::read_dir(paths.machines_root())? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = match entry.file_name().into_string() {
            Ok(value) => value,
            Err(_) => continue,
        };
        if validate_machine_name(&name).is_err() {
            continue;
        }
        match load_machine(paths, &name) {
            Ok(record) => records.push(record),
            Err(error) => {
                records.push(MachineRecord {
                    schema_version: MACHINE_SCHEMA_VERSION,
                    name,
                    architecture: "unknown".to_owned(),
                    mode: crate::model::MachineMode::Persistent,
                    state: MachineState::Stale,
                    transport: crate::model::VirtioTransport::Pci,
                    vcpu_count: 0,
                    memory_mib: 0,
                    boot_args: String::new(),
                    firecracker: unknown_asset(),
                    kernel: unknown_asset(),
                    rootfs_base: unknown_asset(),
                    disks: Vec::new(),
                    network: crate::model::NetworkRecord {
                        tap_name: String::new(),
                        guest_mac: String::new(),
                        guest_address: String::new(),
                        gateway_address: String::new(),
                        prefix_length: 0,
                        dns_servers: Vec::new(),
                        published_ports: Vec::new(),
                        managed: false,
                    },
                    ssh_user: "root".to_owned(),
                    ssh_key_path: String::new(),
                    api_socket: String::new(),
                    config_path: String::new(),
                    serial_log_path: String::new(),
                    process: None,
                    created_at_unix_ms: 0,
                    updated_at_unix_ms: 0,
                    last_error: Some(crate::model::TypedError::new("CORRUPT_MACHINE_RECORD", error.to_string())),
                    raw: Default::default(),
                });
            }
        }
    }
    records.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(records)
}

fn unknown_asset() -> crate::model::AssetIdentity {
    crate::model::AssetIdentity {
        path: String::new(),
        sha256: String::new(),
        version: "unknown".to_owned(),
        provenance_path: None,
    }
}

pub fn load_request(paths: &RuntimePaths, request_id: &str) -> Result<Option<RequestRecord>> {
    let path = paths.request_path(request_id)?;
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(read_json(&path)?))
}

pub fn save_request(paths: &RuntimePaths, record: &RequestRecord) -> Result<()> {
    atomic_write_json(&paths.request_path(&record.request_id)?, record, 0o600)
}

pub fn load_result(paths: &RuntimePaths, handle: &str) -> Result<ResultRecord> {
    let path = paths.result_dir(handle)?.join("result.json");
    read_json(&path)
}

pub fn save_result(paths: &RuntimePaths, record: &ResultRecord) -> Result<()> {
    let path = paths.result_dir(&record.handle)?.join("result.json");
    atomic_write_json(&path, record, 0o600)
}

pub fn remove_machine_dir(paths: &RuntimePaths, name: &str) -> Result<()> {
    let directory = paths.machine_dir(name)?;
    if directory.exists() {
        fs::remove_dir_all(&directory)?;
    }
    Ok(())
}

pub fn safe_remove_file(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn validate_record_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("invalid record identifier");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_path_cannot_escape_root() {
        let paths = RuntimePaths {
            state_root: PathBuf::from("/tmp/smp-test"),
            ..RuntimePaths::default()
        };
        assert!(paths.machine_dir("../escape").is_err());
        assert_eq!(paths.machine_dir("good").unwrap(), PathBuf::from("/tmp/smp-test/machines/good"));
    }

    #[test]
    fn record_ids_are_bounded() {
        assert!(validate_record_id("request-123").is_ok());
        assert!(validate_record_id("../../etc").is_err());
    }
}
