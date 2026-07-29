use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const MACHINE_SCHEMA_VERSION: u32 = 1;
pub const REQUEST_SCHEMA_VERSION: u32 = 1;
pub const RESPONSE_SCHEMA_VERSION: u32 = 1;
pub const SMP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD_COMMIT: &str = env!("SMP_BUILD_COMMIT");
pub const FIRECRACKER_VERSION: &str = "1.15.1";
pub const KERNEL_VERSION: &str = "6.1.177";
pub const DEBIAN_SUITE: &str = "trixie";
pub const DEBIAN_VERSION: &str = "13.6";
pub const INLINE_OUTPUT_LIMIT: u64 = 1_048_576;
pub const CAPTURE_OUTPUT_LIMIT: u64 = 67_108_864;
pub const MAX_TIMEOUT_SECONDS: u64 = 86_400;
pub const REQUEST_RETENTION_SECONDS: u64 = 604_800;
pub const RESULT_RETENTION_SECONDS: u64 = 604_800;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MachineMode {
    Persistent,
    Disposable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VirtioTransport {
    Pci,
    Mmio,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProcessIdentity {
    pub pid: u32,
    pub start_time_ticks: u64,
    pub executable: String,
    pub executable_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DiskRecord {
    pub drive_id: String,
    pub path: String,
    pub logical_size_bytes: u64,
    pub filesystem_uuid: Option<String>,
    pub writable: bool,
    pub attached: bool,
    pub base_image_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct NetworkRecord {
    pub tap_name: String,
    pub guest_mac: String,
    pub guest_address: String,
    pub gateway_address: String,
    pub prefix_length: u8,
    pub dns_servers: Vec<String>,
    pub published_ports: Vec<PublishedPort>,
    pub managed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PublishedPort {
    pub protocol: String,
    pub host_port: u16,
    pub guest_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AssetIdentity {
    pub path: String,
    pub sha256: String,
    pub version: String,
    pub provenance_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MachineRecord {
    pub schema_version: u32,
    pub name: String,
    pub architecture: String,
    pub mode: MachineMode,
    pub state: MachineState,
    pub transport: VirtioTransport,
    pub vcpu_count: u8,
    pub memory_mib: u32,
    pub boot_args: String,
    pub firecracker: AssetIdentity,
    pub kernel: AssetIdentity,
    pub rootfs_base: AssetIdentity,
    pub disks: Vec<DiskRecord>,
    pub network: NetworkRecord,
    pub ssh_user: String,
    pub ssh_key_path: String,
    pub api_socket: String,
    pub config_path: String,
    pub serial_log_path: String,
    pub process: Option<ProcessIdentity>,
    pub created_at_unix_ms: u128,
    pub updated_at_unix_ms: u128,
    pub last_error: Option<TypedError>,
    #[serde(default)]
    pub raw: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GoRequest {
    pub schema_version: u32,
    pub request_id: String,
    pub operation: String,
    #[serde(default)]
    pub machine: Option<String>,
    #[serde(default)]
    pub argv: Vec<String>,
    #[serde(default)]
    pub stdin: Option<String>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub output_limit_bytes: Option<u64>,
    #[serde(default)]
    pub detach: bool,
    #[serde(default)]
    pub options: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoResponse {
    pub schema_version: u32,
    pub request_id: String,
    pub state: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stdout: String,
    pub stderr: String,
    pub stdout_complete: bool,
    pub stderr_complete: bool,
    pub capture_exhausted: bool,
    pub result_handle: Option<String>,
    pub machine_state: Option<MachineState>,
    pub error: Option<TypedError>,
    #[serde(default)]
    pub data: Value,
}

impl GoResponse {
    pub fn completed(request_id: impl Into<String>, data: Value) -> Self {
        Self {
            schema_version: RESPONSE_SCHEMA_VERSION,
            request_id: request_id.into(),
            state: "completed".to_owned(),
            exit_code: Some(0),
            timed_out: false,
            cancelled: false,
            stdout: String::new(),
            stderr: String::new(),
            stdout_complete: true,
            stderr_complete: true,
            capture_exhausted: false,
            result_handle: None,
            machine_state: None,
            error: None,
            data,
        }
    }

    pub fn failed(request_id: impl Into<String>, error: TypedError) -> Self {
        Self {
            schema_version: RESPONSE_SCHEMA_VERSION,
            request_id: request_id.into(),
            state: "failed".to_owned(),
            exit_code: None,
            timed_out: false,
            cancelled: false,
            stdout: String::new(),
            stderr: String::new(),
            stdout_complete: true,
            stderr_complete: true,
            capture_exhausted: false,
            result_handle: None,
            machine_state: None,
            error: Some(error),
            data: Value::Null,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TypedError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(default)]
    pub details: Value,
}

impl TypedError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
            details: Value::Null,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestRecord {
    pub schema_version: u32,
    pub request_id: String,
    pub request_digest: String,
    pub operation: String,
    pub machine: Option<String>,
    pub state: String,
    pub result_handle: Option<String>,
    pub process: Option<ProcessIdentity>,
    pub created_at_unix_ms: u128,
    pub updated_at_unix_ms: u128,
    pub terminal_response: Option<GoResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultRecord {
    pub schema_version: u32,
    pub handle: String,
    pub request_id: String,
    pub state: String,
    pub process: Option<ProcessIdentity>,
    pub stdout_path: String,
    pub stderr_path: String,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub stdout_complete: bool,
    pub stderr_complete: bool,
    pub capture_exhausted: bool,
    pub exit_code: Option<i32>,
    pub cancelled: bool,
    pub created_at_unix_ms: u128,
    pub updated_at_unix_ms: u128,
    pub error: Option<TypedError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationSchema {
    pub name: String,
    pub summary: String,
    pub arguments: Value,
}
