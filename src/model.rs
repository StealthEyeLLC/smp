use crate::error::{Result, SmpError};
use crate::paths::validate_machine_name;
use crate::{MACHINE_SCHEMA_VERSION, REQUEST_SCHEMA_VERSION, RESPONSE_SCHEMA_VERSION};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineMode {
    Persistent,
    Disposable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Pci,
    Mmio,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineState {
    Absent,
    Created,
    Starting,
    Running,
    Ready,
    Stopped,
    Crashed,
    Stale,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortProtocol {
    Tcp,
    Udp,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PortPublication {
    pub protocol: PortProtocol,
    pub bind_address: String,
    pub host_port: u16,
    pub guest_port: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkDefinition {
    pub tap: String,
    pub subnet: String,
    pub prefix_length: u8,
    pub guest_address: String,
    pub gateway: String,
    pub dns: Vec<String>,
    pub guest_mac: String,
    #[serde(default)]
    pub published_ports: Vec<PortPublication>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiskAttachment {
    pub id: String,
    pub path: PathBuf,
    pub digest: Option<String>,
    pub filesystem_uuid: Option<String>,
    pub logical_size: u64,
    pub physical_size: u64,
    pub read_only: bool,
    pub is_root: bool,
    pub active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessIdentity {
    pub pid: i32,
    pub process_start_time: u64,
    pub executable_path: PathBuf,
    pub executable_digest: Option<String>,
    pub boot_id: String,
    pub process_group: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MachineRecord {
    pub schema_version: u32,
    pub machine_id: String,
    pub mode: MachineMode,
    pub architecture: String,
    pub transport: Transport,
    pub vcpu_count: u8,
    pub memory_mib: u32,
    pub firecracker_path: PathBuf,
    pub firecracker_digest: String,
    pub kernel_path: PathBuf,
    pub kernel_digest: String,
    pub kernel_arguments: String,
    pub initrd_path: Option<PathBuf>,
    pub root_disk: DiskAttachment,
    #[serde(default)]
    pub additional_disks: Vec<DiskAttachment>,
    pub base_image_digest: String,
    pub network: NetworkDefinition,
    pub seed_path: PathBuf,
    pub seed_identity: String,
    pub machine_directory: PathBuf,
    pub api_socket: PathBuf,
    pub firecracker_process: Option<ProcessIdentity>,
    pub generated_config_digest: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub state: MachineState,
    pub last_error: Option<String>,
}

impl MachineRecord {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != MACHINE_SCHEMA_VERSION {
            return Err(SmpError::State(format!(
                "unsupported machine schema {}; supported {}",
                self.schema_version, MACHINE_SCHEMA_VERSION
            )));
        }
        validate_machine_name(&self.machine_id)?;
        if self.architecture != "x86_64" {
            return Err(SmpError::State(format!(
                "unsupported architecture {}",
                self.architecture
            )));
        }
        if self.vcpu_count == 0 || self.memory_mib < 64 {
            return Err(SmpError::Invalid(
                "machine must have at least one vCPU and 64 MiB memory".to_owned(),
            ));
        }
        if self.network.prefix_length > 32 {
            return Err(SmpError::Invalid("invalid IPv4 prefix length".to_owned()));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteRequest {
    pub schema_version: u32,
    pub request_id: String,
    pub operation: String,
    pub machine: Option<String>,
    pub argv: Option<Vec<String>>,
    pub stdin: Option<String>,
    pub timeout_seconds: Option<u64>,
    pub output_limit_bytes: Option<u64>,
    #[serde(default)]
    pub detach: bool,
    #[serde(default)]
    pub options: Map<String, Value>,
}

impl RemoteRequest {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != REQUEST_SCHEMA_VERSION {
            return Err(SmpError::Invalid(format!(
                "unsupported request schema {}; supported {}",
                self.schema_version, REQUEST_SCHEMA_VERSION
            )));
        }
        if self.request_id.is_empty() || self.request_id.len() > 128 {
            return Err(SmpError::Invalid("invalid requestId length".to_owned()));
        }
        if self.operation.is_empty() || self.operation.len() > 128 {
            return Err(SmpError::Invalid("invalid operation".to_owned()));
        }
        if let Some(machine) = &self.machine {
            validate_machine_name(machine)?;
        }
        if self.argv.as_ref().is_some_and(Vec::is_empty) {
            return Err(SmpError::Invalid("argv cannot be empty".to_owned()));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultState {
    Accepted,
    Running,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
    Stale,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteResponse {
    pub schema_version: u32,
    pub request_id: String,
    pub state: ResultState,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
    pub stdout_complete: bool,
    pub stderr_complete: bool,
    pub result_handle: Option<String>,
    pub machine_state: Option<MachineState>,
    pub result: Option<Value>,
    pub error: Option<String>,
}

impl RemoteResponse {
    pub fn completed(request_id: impl Into<String>, result: Value) -> Self {
        Self {
            schema_version: RESPONSE_SCHEMA_VERSION,
            request_id: request_id.into(),
            state: ResultState::Completed,
            exit_code: Some(0),
            timed_out: false,
            stdout: String::new(),
            stderr: String::new(),
            stdout_complete: true,
            stderr_complete: true,
            result_handle: None,
            machine_state: None,
            result: Some(result),
            error: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestRecord {
    pub schema_version: u32,
    pub request_id: String,
    pub request_digest: String,
    pub operation: String,
    pub state: ResultState,
    pub process: Option<ProcessIdentity>,
    pub result_directory: Option<PathBuf>,
    pub response: Option<RemoteResponse>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_machine_schema_is_rejected() -> Result<()> {
        let value = serde_json::json!({
            "schemaVersion": 999,
            "machineId": "default",
            "mode": "persistent",
            "architecture": "x86_64",
            "transport": "pci",
            "vcpuCount": 1,
            "memoryMib": 128,
            "firecrackerPath": "/x",
            "firecrackerDigest": "x",
            "kernelPath": "/k",
            "kernelDigest": "k",
            "kernelArguments": "",
            "initrdPath": null,
            "rootDisk": {"id":"root","path":"/r","digest":null,"filesystemUuid":null,"logicalSize":1,"physicalSize":1,"readOnly":false,"isRoot":true,"active":false},
            "additionalDisks": [],
            "baseImageDigest": "b",
            "network": {"tap":"smp0","subnet":"172.31.1.0","prefixLength":30,"guestAddress":"172.31.1.2","gateway":"172.31.1.1","dns":["1.1.1.1"],"guestMac":"06:00:00:00:00:01","publishedPorts":[]},
            "seedPath": "/s",
            "seedIdentity": "s",
            "machineDirectory": "/m",
            "apiSocket": "/a",
            "firecrackerProcess": null,
            "generatedConfigDigest": null,
            "createdAt": 1,
            "updatedAt": 1,
            "state": "created",
            "lastError": null
        });
        let record: MachineRecord =
            serde_json::from_value(value).map_err(|error| SmpError::json("<test>", error))?;
        assert!(record.validate().is_err());
        Ok(())
    }

    #[test]
    fn unknown_machine_fields_are_rejected() {
        let result = serde_json::from_value::<NetworkDefinition>(serde_json::json!({
            "tap":"smp0","subnet":"172.31.1.0","prefixLength":30,
            "guestAddress":"172.31.1.2","gateway":"172.31.1.1",
            "dns":[],"guestMac":"06:00:00:00:00:01",
            "publishedPorts":[],"unexpected":true
        }));
        assert!(result.is_err());
    }
}
