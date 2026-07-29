use crate::assets;
use crate::doctor;
use crate::error::{Result, SmpError};
use crate::firecracker;
use crate::guest;
use crate::machine::{CreateOptions, Manager};
use crate::model::{
    MachineMode, RemoteRequest, RemoteResponse, RequestRecord, ResultState, Transport,
};
use crate::paths::{Paths, validate_record_id};
use crate::process;
use crate::util::{
    atomic_json, atomic_write, canonical_json_digest, now_unix_seconds, read_json, sha256_file,
};
use crate::{BUILD_COMMIT, REQUEST_SCHEMA_VERSION, RESPONSE_SCHEMA_VERSION, VERSION};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use fs2::FileExt;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const INLINE_OUTPUT_LIMIT: u64 = 1024 * 1024;
pub const TOTAL_CAPTURE_LIMIT: u64 = 64 * 1024 * 1024;
pub const MAX_TIMEOUT_SECONDS: u64 = 86_400;
pub const REQUEST_RETENTION_SECONDS: u64 = 86_400;
pub const RESULT_RETENTION_SECONDS: u64 = 86_400;
const RESULT_CHUNK_LIMIT: usize = 1024 * 1024;

#[derive(Clone)]
pub struct Engine {
    pub paths: Paths,
    manager: Manager,
}

impl Engine {
    pub fn new(paths: Paths) -> Self {
        Self {
            manager: Manager::new(paths.clone()),
            paths,
        }
    }

    pub fn handle(&self, request: RemoteRequest) -> Result<RemoteResponse> {
        request.validate()?;
        self.paths.ensure_layout()?;
        let _lock = self.request_lock(&request.request_id)?;
        let digest = canonical_json_digest(&request)?;
        let record_path = self.paths.request_record(&request.request_id)?;
        if record_path.exists() {
            let record: RequestRecord = read_json(&record_path)?;
            if record.request_digest != digest {
                return Err(SmpError::Conflict(format!(
                    "request ID {} already has a different digest",
                    request.request_id
                )));
            }
            return Ok(record
                .response
                .unwrap_or_else(|| running_response(&request.request_id)));
        }

        let now = now_unix_seconds();
        let mut record = RequestRecord {
            schema_version: REQUEST_SCHEMA_VERSION,
            request_id: request.request_id.clone(),
            request_digest: digest,
            normalized_request: request.clone(),
            operation: request.operation.clone(),
            state: ResultState::Accepted,
            process: None,
            result_directory: Some(self.paths.result_dir(&request.request_id)?),
            response: None,
            created_at: now,
            updated_at: now,
        };
        atomic_json(&record_path, &record, 0o600)?;

        if request.detach {
            match self.start_detached(&mut record) {
                Ok(response) => {
                    atomic_json(&record_path, &record, 0o600)?;
                    return Ok(response);
                }
                Err(error) => {
                    let response = failed_response(&request.request_id, &error);
                    record.state = ResultState::Failed;
                    record.response = Some(response.clone());
                    record.updated_at = now_unix_seconds();
                    atomic_json(&record_path, &record, 0o600)?;
                    return Err(error);
                }
            }
        }

        let response = match self.dispatch(&request) {
            Ok(response) => response,
            Err(error) => failed_response(&request.request_id, &error),
        };
        record.state = response.state.clone();
        record.response = Some(response.clone());
        record.updated_at = now_unix_seconds();
        atomic_json(&record_path, &record, 0o600)?;
        Ok(response)
    }

    pub fn run_detached(&self, request_id: &str) -> Result<i32> {
        validate_record_id(request_id)?;
        thread::sleep(Duration::from_millis(100));
        let _lock = self.request_lock(request_id)?;
        let record_path = self.paths.request_record(request_id)?;
        let mut record: RequestRecord = read_json(&record_path)?;
        if record.state != ResultState::Running {
            return Err(SmpError::Conflict(format!(
                "detached request {request_id} is not running"
            )));
        }
        let mut request = record.normalized_request.clone();
        request.detach = false;
        let response = match self.dispatch(&request) {
            Ok(response) => response,
            Err(error) => failed_response(request_id, &error),
        };
        let exit_code = response.exit_code.unwrap_or(1);
        record.state = response.state.clone();
        record.response = Some(response);
        record.updated_at = now_unix_seconds();
        atomic_json(&record_path, &record, 0o600)?;
        Ok(exit_code)
    }

    pub fn describe(&self, include_machines: bool) -> Result<Value> {
        let verified = assets::verify(&self.paths).ok();
        let binary_digest = if self.paths.binary.is_file() {
            sha256_file(&self.paths.binary).ok()
        } else {
            None
        };
        let machines = if include_machines {
            Value::Array(
                self.manager
                    .list()?
                    .into_iter()
                    .map(|record| {
                        json!({
                            "machineId": record.machine_id,
                            "state": record.state,
                            "mode": record.mode,
                            "transport": record.transport
                        })
                    })
                    .collect(),
            )
        } else {
            Value::Null
        };
        Ok(json!({
            "product": "SMP",
            "version": VERSION,
            "buildCommit": BUILD_COMMIT,
            "installedBinarySha256": binary_digest,
            "requestSchemaVersion": REQUEST_SCHEMA_VERSION,
            "responseSchemaVersion": RESPONSE_SCHEMA_VERSION,
            "operationCatalog": operation_catalog(),
            "operationSchemas": operation_schemas(),
            "firecracker": verified.as_ref().map(|value| &value.manifest.firecracker),
            "kernel": verified.as_ref().map(|value| &value.manifest.kernel),
            "rootfs": verified.as_ref().map(|value| &value.manifest.debian),
            "assetManifestSha256": verified.as_ref().map(|value| &value.manifest_digest),
            "hostArchitecture": std::env::consts::ARCH,
            "serverInstance": server_instance(),
            "transportCapabilities": ["pci", "mmio"],
            "limits": {
                "inlineOutputBytes": INLINE_OUTPUT_LIMIT,
                "totalCaptureBytes": TOTAL_CAPTURE_LIMIT,
                "timeoutSeconds": MAX_TIMEOUT_SECONDS,
                "requestRetentionSeconds": REQUEST_RETENTION_SECONDS,
                "resultRetentionSeconds": RESULT_RETENTION_SECONDS
            },
            "directories": self.paths.directory_contract(),
            "machines": machines
        }))
    }

    pub fn reconcile_all(&self) -> Result<Vec<Value>> {
        self.manager
            .list()?
            .into_iter()
            .map(|record| {
                self.manager
                    .reconcile(&record.machine_id)
                    .and_then(|updated| {
                        serde_json::to_value(updated)
                            .map_err(|error| SmpError::json("<reconcile>", error))
                    })
            })
            .collect()
    }

    fn dispatch(&self, request: &RemoteRequest) -> Result<RemoteResponse> {
        let machine = request.machine.as_deref().unwrap_or("default");
        match request.operation.as_str() {
            "describe" => Ok(RemoteResponse::completed(
                &request.request_id,
                self.describe(option_bool(&request.options, "includeMachines", false)?)?,
            )),
            "doctor" => Ok(RemoteResponse::completed(
                &request.request_id,
                serde_json::to_value(doctor::inspect(&self.paths))
                    .map_err(|error| SmpError::json("<doctor>", error))?,
            )),
            "machine.create" => {
                let record = self.manager.create(create_options(request, machine)?)?;
                completed_serialized(&request.request_id, &record)
            }
            "machine.start" => {
                let record = self.manager.start(machine, timeout(request, 300)?)?;
                completed_serialized(&request.request_id, &record)
            }
            "machine.wait" => {
                let record = self.manager.wait(machine, timeout(request, 300)?)?;
                completed_serialized(&request.request_id, &record)
            }
            "machine.status" | "machine.inspect" => {
                let record = self.manager.load(machine)?;
                completed_serialized(&request.request_id, &record)
            }
            "machine.stop" => {
                let record = self.manager.stop(machine, timeout(request, 60)?)?;
                completed_serialized(&request.request_id, &record)
            }
            "machine.kill" => {
                let record = self.manager.kill(machine)?;
                completed_serialized(&request.request_id, &record)
            }
            "machine.reboot" => {
                let record = self.manager.reboot(machine, timeout(request, 300)?)?;
                completed_serialized(&request.request_id, &record)
            }
            "machine.destroy" => {
                self.manager
                    .destroy(machine, option_bool(&request.options, "deleteDisk", false)?)?;
                Ok(RemoteResponse::completed(
                    &request.request_id,
                    json!({"destroyed": machine}),
                ))
            }
            "machine.reconcile" => {
                let record = self.manager.reconcile(machine)?;
                completed_serialized(&request.request_id, &record)
            }
            "exec" => self.execute_guest(request, machine),
            "file.upload" => self.upload(request, machine),
            "file.download" => self.download(request, machine),
            "logs.read" => self.read_log(request, machine),
            "raw.smp" => self.raw_smp(request),
            "raw.firecracker" => self.raw_firecracker(request, machine),
            "result.get" => self.result_get(request),
            "result.read" => self.result_read(request),
            "result.wait" => self.result_wait(request),
            "result.cancel" => self.result_cancel(request),
            operation => Err(SmpError::Invalid(format!(
                "unsupported operation {operation}; call describe for the live catalog"
            ))),
        }
    }

    fn execute_guest(&self, request: &RemoteRequest, machine: &str) -> Result<RemoteResponse> {
        let argv = request
            .argv
            .as_ref()
            .ok_or_else(|| SmpError::Invalid("exec requires argv".to_owned()))?;
        let record = self.manager.load(machine)?;
        let stdin = request.stdin.as_deref().map(decode_base64).transpose()?;
        let result = guest::execute(
            &record,
            &self.manager.ssh_key(),
            argv,
            stdin.as_deref(),
            timeout(request, 300)?,
            capture_limit(request)?,
            option_bool(&request.options, "tty", false)?,
        )?;
        self.execution_response(request, result)
    }

    fn upload(&self, request: &RemoteRequest, machine: &str) -> Result<RemoteResponse> {
        let data = decode_base64(&option_string(&request.options, "dataBase64")?)?;
        if u64::try_from(data.len()).unwrap_or(u64::MAX) > TOTAL_CAPTURE_LIMIT {
            return Err(SmpError::Invalid(
                "upload exceeds total capture limit".to_owned(),
            ));
        }
        let destination = PathBuf::from(option_string(&request.options, "destination")?);
        let directory = self.paths.result_dir(&request.request_id)?;
        fs::create_dir_all(&directory)
            .map_err(|error| SmpError::io(directory.display().to_string(), error))?;
        let source = directory.join("upload.bin");
        atomic_write(&source, &data, 0o600)?;
        let record = self.manager.load(machine)?;
        guest::upload(&record, &self.manager.ssh_key(), &source, &destination)?;
        Ok(RemoteResponse::completed(
            &request.request_id,
            json!({"bytes": data.len(), "sha256": sha256_file(&source)?}),
        ))
    }

    fn download(&self, request: &RemoteRequest, machine: &str) -> Result<RemoteResponse> {
        let source = PathBuf::from(option_string(&request.options, "source")?);
        let directory = self.paths.result_dir(&request.request_id)?;
        fs::create_dir_all(&directory)
            .map_err(|error| SmpError::io(directory.display().to_string(), error))?;
        let destination = directory.join("download.bin");
        let record = self.manager.load(machine)?;
        guest::download(&record, &self.manager.ssh_key(), &source, &destination)?;
        let bytes = fs::read(&destination)
            .map_err(|error| SmpError::io(destination.display().to_string(), error))?;
        let mut response = RemoteResponse::completed(
            &request.request_id,
            json!({"bytes": bytes.len(), "sha256": sha256_file(&destination)?}),
        );
        response.total_stdout_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if response.total_stdout_bytes <= INLINE_OUTPUT_LIMIT {
            response.stdout = STANDARD.encode(bytes);
        } else {
            response.stdout_complete = false;
            response.result_handle = Some(request.request_id.clone());
        }
        Ok(response)
    }

    fn read_log(&self, request: &RemoteRequest, machine: &str) -> Result<RemoteResponse> {
        let stream = option_string_default(&request.options, "stream", "stdout")?;
        let offset = option_u64(&request.options, "offset", 0)?;
        let limit = option_u64(&request.options, "limit", 64 * 1024)?
            .min(u64::try_from(RESULT_CHUNK_LIMIT).unwrap_or(u64::MAX));
        let chunk = self.manager.read_log(
            machine,
            &stream,
            offset,
            usize::try_from(limit).unwrap_or(RESULT_CHUNK_LIMIT),
        )?;
        completed_serialized(&request.request_id, &chunk)
    }

    fn raw_smp(&self, request: &RemoteRequest) -> Result<RemoteResponse> {
        let argv = request
            .argv
            .as_ref()
            .ok_or_else(|| SmpError::Invalid("raw.smp requires argv".to_owned()))?;
        if argv.first().is_some_and(|value| value.starts_with("__")) {
            return Err(SmpError::Invalid(
                "internal SMP operations are not remotely callable".to_owned(),
            ));
        }
        let executable = process::current_executable()?;
        let output = Command::new(&executable)
            .args(argv)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| SmpError::io(executable.display().to_string(), error))?;
        let result = guest::ExecutionResult {
            exit_code: output.status.code().unwrap_or(128),
            signal: None,
            timed_out: false,
            total_stdout_bytes: u64::try_from(output.stdout.len()).unwrap_or(u64::MAX),
            total_stderr_bytes: u64::try_from(output.stderr.len()).unwrap_or(u64::MAX),
            stdout_complete: true,
            stderr_complete: true,
            stdout: output.stdout,
            stderr: output.stderr,
        };
        self.execution_response(request, result)
    }

    fn raw_firecracker(&self, request: &RemoteRequest, machine: &str) -> Result<RemoteResponse> {
        let record = self.manager.load(machine)?;
        let method = option_string_default(&request.options, "method", "GET")?;
        let path = option_string(&request.options, "path")?;
        let headers = request
            .options
            .get("headers")
            .cloned()
            .map(serde_json::from_value::<BTreeMap<String, String>>)
            .transpose()
            .map_err(|error| SmpError::json("<raw.firecracker headers>", error))?
            .unwrap_or_default();
        let body = request
            .options
            .get("bodyBase64")
            .and_then(Value::as_str)
            .map(decode_base64)
            .transpose()?
            .unwrap_or_default();
        let response = firecracker::raw_api(&record, &method, &path, &headers, &body)?;
        Ok(RemoteResponse::completed(
            &request.request_id,
            json!({
                "statusCode": response.status_code,
                "headers": response.headers,
                "bodyBase64": STANDARD.encode(response.body)
            }),
        ))
    }

    fn result_get(&self, request: &RemoteRequest) -> Result<RemoteResponse> {
        let target = option_string(&request.options, "requestId")?;
        let record: RequestRecord = read_json(&self.paths.request_record(&target)?)?;
        Ok(record
            .response
            .unwrap_or_else(|| running_response(&request.request_id)))
    }

    fn result_read(&self, request: &RemoteRequest) -> Result<RemoteResponse> {
        let target = option_string(&request.options, "requestId")?;
        let stream = option_string_default(&request.options, "stream", "stdout")?;
        if !matches!(stream.as_str(), "stdout" | "stderr" | "download") {
            return Err(SmpError::Invalid("invalid result stream".to_owned()));
        }
        let offset = option_u64(&request.options, "offset", 0)?;
        let limit = option_u64(&request.options, "limit", 64 * 1024)?
            .min(u64::try_from(RESULT_CHUNK_LIMIT).unwrap_or(u64::MAX));
        let name = if stream == "download" {
            "download.bin"
        } else if stream == "stdout" {
            "stdout.bin"
        } else {
            "stderr.bin"
        };
        let path = self.paths.result_dir(&target)?.join(name);
        let chunk = read_chunk(
            &path,
            offset,
            usize::try_from(limit).unwrap_or(RESULT_CHUNK_LIMIT),
        )?;
        Ok(RemoteResponse::completed(&request.request_id, chunk))
    }

    fn result_wait(&self, request: &RemoteRequest) -> Result<RemoteResponse> {
        let target = option_string(&request.options, "requestId")?;
        let deadline = Instant::now() + timeout(request, 300)?;
        loop {
            let record: RequestRecord = read_json(&self.paths.request_record(&target)?)?;
            if let Some(response) = record.response {
                return Ok(response);
            }
            if let Some(identity) = record.process.as_ref()
                && !process::is_running(identity)
            {
                return Err(SmpError::State(format!(
                    "detached request {target} exited without terminal metadata"
                )));
            }
            if Instant::now() >= deadline {
                return Err(SmpError::State(format!(
                    "timed out waiting for request {target}"
                )));
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn result_cancel(&self, request: &RemoteRequest) -> Result<RemoteResponse> {
        let target = option_string(&request.options, "requestId")?;
        let _lock = self.request_lock(&target)?;
        let path = self.paths.request_record(&target)?;
        let mut record: RequestRecord = read_json(&path)?;
        let identity = record
            .process
            .as_ref()
            .ok_or_else(|| SmpError::Conflict(format!("request {target} has no active process")))?;
        process::signal(identity, libc::SIGTERM)?;
        if !process::wait_for_exit(identity, Duration::from_secs(10))? {
            process::signal(identity, libc::SIGKILL)?;
        }
        let mut response = failed_response(&target, &SmpError::State("cancelled".to_owned()));
        response.state = ResultState::Cancelled;
        response.error = None;
        record.state = ResultState::Cancelled;
        record.response = Some(response.clone());
        record.updated_at = now_unix_seconds();
        atomic_json(&path, &record, 0o600)?;
        Ok(response)
    }

    fn execution_response(
        &self,
        request: &RemoteRequest,
        result: guest::ExecutionResult,
    ) -> Result<RemoteResponse> {
        let directory = self.paths.result_dir(&request.request_id)?;
        fs::create_dir_all(&directory)
            .map_err(|error| SmpError::io(directory.display().to_string(), error))?;
        atomic_write(&directory.join("stdout.bin"), &result.stdout, 0o600)?;
        atomic_write(&directory.join("stderr.bin"), &result.stderr, 0o600)?;
        let inline_limit = request
            .output_limit_bytes
            .unwrap_or(INLINE_OUTPUT_LIMIT)
            .min(INLINE_OUTPUT_LIMIT);
        let total_retained =
            u64::try_from(result.stdout.len() + result.stderr.len()).unwrap_or(u64::MAX);
        let inline = total_retained <= inline_limit;
        Ok(RemoteResponse {
            schema_version: RESPONSE_SCHEMA_VERSION,
            request_id: request.request_id.clone(),
            state: if result.timed_out {
                ResultState::TimedOut
            } else if result.exit_code == 0 {
                ResultState::Completed
            } else {
                ResultState::Failed
            },
            exit_code: Some(result.exit_code),
            timed_out: result.timed_out,
            stdout: if inline {
                STANDARD.encode(&result.stdout)
            } else {
                String::new()
            },
            stderr: if inline {
                STANDARD.encode(&result.stderr)
            } else {
                String::new()
            },
            output_encoding: "base64".to_owned(),
            total_stdout_bytes: result.total_stdout_bytes,
            total_stderr_bytes: result.total_stderr_bytes,
            stdout_complete: inline && result.stdout_complete,
            stderr_complete: inline && result.stderr_complete,
            result_handle: (!inline).then(|| request.request_id.clone()),
            machine_state: None,
            result: Some(json!({"signal": result.signal})),
            error: None,
        })
    }

    fn start_detached(&self, record: &mut RequestRecord) -> Result<RemoteResponse> {
        if !matches!(record.operation.as_str(), "exec" | "raw.smp") {
            return Err(SmpError::Invalid(format!(
                "detach is not supported for {}",
                record.operation
            )));
        }
        let directory = self.paths.result_dir(&record.request_id)?;
        fs::create_dir_all(&directory)
            .map_err(|error| SmpError::io(directory.display().to_string(), error))?;
        let executable = process::current_executable()?;
        let args = vec!["__remote-worker".to_owned(), record.request_id.clone()];
        let (_child, identity) = process::spawn_detached(
            &executable,
            &args,
            &directory,
            &directory.join("worker.stdout.log"),
            &directory.join("worker.stderr.log"),
        )?;
        record.state = ResultState::Running;
        record.process = Some(identity);
        record.updated_at = now_unix_seconds();
        Ok(running_response(&record.request_id))
    }

    fn request_lock(&self, request_id: &str) -> Result<File> {
        validate_record_id(request_id)?;
        fs::create_dir_all(&self.paths.runtime)
            .map_err(|error| SmpError::io(self.paths.runtime.display().to_string(), error))?;
        let path = self
            .paths
            .runtime
            .join(format!("request-{request_id}.lock"));
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
}

pub fn operation_catalog() -> Vec<&'static str> {
    vec![
        "describe",
        "doctor",
        "machine.create",
        "machine.start",
        "machine.wait",
        "machine.status",
        "machine.inspect",
        "machine.stop",
        "machine.kill",
        "machine.reboot",
        "machine.destroy",
        "machine.reconcile",
        "exec",
        "file.upload",
        "file.download",
        "logs.read",
        "raw.smp",
        "raw.firecracker",
        "result.get",
        "result.read",
        "result.wait",
        "result.cancel",
    ]
}

pub fn operation_schemas() -> Value {
    json!({
        "machine.create": {"options": {"mode": "persistent|disposable", "transport": "pci|mmio", "vcpuCount": "integer", "memoryMib": "integer"}},
        "exec": {"argv": "exact string array", "stdin": "base64", "detach": "boolean"},
        "file.upload": {"options": {"destination": "absolute guest path", "dataBase64": "string"}},
        "file.download": {"options": {"source": "absolute guest path"}},
        "logs.read": {"options": {"stream": "stdout|stderr|serial", "offset": "integer", "limit": "integer"}},
        "raw.smp": {"argv": "exact SMP argv"},
        "raw.firecracker": {"options": {"method": "GET|PUT|PATCH|DELETE", "path": "managed API path", "headers": "object", "bodyBase64": "string"}},
        "result.get|read|wait|cancel": {"options": {"requestId": "string"}}
    })
}

fn create_options(request: &RemoteRequest, machine: &str) -> Result<CreateOptions> {
    let mode = match option_string_default(&request.options, "mode", "persistent")?.as_str() {
        "persistent" => MachineMode::Persistent,
        "disposable" => MachineMode::Disposable,
        value => return Err(SmpError::Invalid(format!("invalid machine mode {value}"))),
    };
    let transport = match option_string_default(&request.options, "transport", "pci")?.as_str() {
        "pci" => Transport::Pci,
        "mmio" => Transport::Mmio,
        value => return Err(SmpError::Invalid(format!("invalid transport {value}"))),
    };
    Ok(CreateOptions {
        machine_id: machine.to_owned(),
        mode,
        transport,
        vcpu_count: u8::try_from(option_u64(&request.options, "vcpuCount", 2)?)
            .map_err(|_| SmpError::Invalid("vCPU count exceeds 255".to_owned()))?,
        memory_mib: u32::try_from(option_u64(&request.options, "memoryMib", 1024)?)
            .map_err(|_| SmpError::Invalid("memory MiB exceeds u32".to_owned()))?,
        firecracker: option_path(&request.options, "firecracker"),
        kernel: option_path(&request.options, "kernel"),
        rootfs: option_path(&request.options, "rootfs"),
        initrd: option_path(&request.options, "initrd"),
        kernel_arguments: request
            .options
            .get("kernelArguments")
            .and_then(Value::as_str)
            .map(str::to_owned),
        published_ports: Vec::new(),
        initialization_script: option_path(&request.options, "initializationScript"),
    })
}

fn timeout(request: &RemoteRequest, default: u64) -> Result<Duration> {
    let seconds = request.timeout_seconds.unwrap_or(default);
    if seconds == 0 || seconds > MAX_TIMEOUT_SECONDS {
        return Err(SmpError::Invalid(format!(
            "timeout must be 1 through {MAX_TIMEOUT_SECONDS} seconds"
        )));
    }
    Ok(Duration::from_secs(seconds))
}

fn capture_limit(request: &RemoteRequest) -> Result<u64> {
    let limit = request.output_limit_bytes.unwrap_or(INLINE_OUTPUT_LIMIT);
    if limit == 0 || limit > TOTAL_CAPTURE_LIMIT {
        return Err(SmpError::Invalid(format!(
            "output limit must be 1 through {TOTAL_CAPTURE_LIMIT}"
        )));
    }
    Ok(limit)
}

fn option_string(options: &Map<String, Value>, name: &str) -> Result<String> {
    options
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| SmpError::Invalid(format!("option {name} is required")))
}

fn option_string_default(
    options: &Map<String, Value>,
    name: &str,
    default: &str,
) -> Result<String> {
    match options.get(name) {
        Some(value) => value
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| SmpError::Invalid(format!("option {name} must be a string"))),
        None => Ok(default.to_owned()),
    }
}

fn option_bool(options: &Map<String, Value>, name: &str, default: bool) -> Result<bool> {
    match options.get(name) {
        Some(value) => value
            .as_bool()
            .ok_or_else(|| SmpError::Invalid(format!("option {name} must be boolean"))),
        None => Ok(default),
    }
}

fn option_u64(options: &Map<String, Value>, name: &str, default: u64) -> Result<u64> {
    match options.get(name) {
        Some(value) => value
            .as_u64()
            .ok_or_else(|| SmpError::Invalid(format!("option {name} must be unsigned integer"))),
        None => Ok(default),
    }
}

fn option_path(options: &Map<String, Value>, name: &str) -> Option<PathBuf> {
    options.get(name).and_then(Value::as_str).map(PathBuf::from)
}

fn decode_base64(value: &str) -> Result<Vec<u8>> {
    STANDARD
        .decode(value)
        .map_err(|error| SmpError::Invalid(format!("invalid base64: {error}")))
}

fn completed_serialized<T: Serialize>(request_id: &str, value: &T) -> Result<RemoteResponse> {
    Ok(RemoteResponse::completed(
        request_id,
        serde_json::to_value(value).map_err(|error| SmpError::json("<remote-response>", error))?,
    ))
}

fn running_response(request_id: &str) -> RemoteResponse {
    RemoteResponse {
        schema_version: RESPONSE_SCHEMA_VERSION,
        request_id: request_id.to_owned(),
        state: ResultState::Running,
        exit_code: None,
        timed_out: false,
        stdout: String::new(),
        stderr: String::new(),
        output_encoding: "base64".to_owned(),
        total_stdout_bytes: 0,
        total_stderr_bytes: 0,
        stdout_complete: false,
        stderr_complete: false,
        result_handle: Some(request_id.to_owned()),
        machine_state: None,
        result: None,
        error: None,
    }
}

fn failed_response(request_id: &str, error: &SmpError) -> RemoteResponse {
    RemoteResponse {
        schema_version: RESPONSE_SCHEMA_VERSION,
        request_id: request_id.to_owned(),
        state: ResultState::Failed,
        exit_code: Some(i32::from(error.exit_code())),
        timed_out: false,
        stdout: String::new(),
        stderr: String::new(),
        output_encoding: "base64".to_owned(),
        total_stdout_bytes: 0,
        total_stderr_bytes: 0,
        stdout_complete: true,
        stderr_complete: true,
        result_handle: None,
        machine_state: None,
        result: None,
        error: Some(error.to_string()),
    }
}

fn read_chunk(path: &Path, offset: u64, limit: usize) -> Result<Value> {
    if limit == 0 || limit > RESULT_CHUNK_LIMIT {
        return Err(SmpError::Invalid("invalid result chunk limit".to_owned()));
    }
    let mut file =
        File::open(path).map_err(|error| SmpError::io(path.display().to_string(), error))?;
    let size = file
        .metadata()
        .map_err(|error| SmpError::io(path.display().to_string(), error))?
        .len();
    let effective = offset.min(size);
    file.seek(SeekFrom::Start(effective))
        .map_err(|error| SmpError::io(path.display().to_string(), error))?;
    let mut data = vec![0_u8; limit];
    let count = file
        .read(&mut data)
        .map_err(|error| SmpError::io(path.display().to_string(), error))?;
    data.truncate(count);
    let next = effective.saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
    Ok(json!({
        "offset": effective,
        "nextOffset": next,
        "totalBytes": size,
        "dataBase64": STANDARD.encode(data),
        "eof": next >= size,
        "truncated": offset > size
    }))
}

fn server_instance() -> String {
    fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|value| format!("{}:{}", value.trim(), std::process::id()))
        .unwrap_or_else(|_| format!("unknown:{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: &str) -> RemoteRequest {
        RemoteRequest {
            schema_version: REQUEST_SCHEMA_VERSION,
            request_id: id.to_owned(),
            operation: "describe".to_owned(),
            machine: None,
            argv: None,
            stdin: None,
            timeout_seconds: None,
            output_limit_bytes: None,
            detach: false,
            options: Map::new(),
        }
    }

    #[test]
    fn request_digest_and_replay_are_deterministic() -> Result<()> {
        let directory = tempfile::tempdir().map_err(|error| SmpError::io("tempdir", error))?;
        let engine = Engine::new(Paths::rooted(directory.path())?);
        let first = engine.handle(request("same"))?;
        let second = engine.handle(request("same"))?;
        assert_eq!(first.request_id, second.request_id);
        assert_eq!(first.state, ResultState::Completed);
        Ok(())
    }

    #[test]
    fn conflicting_replay_is_rejected() -> Result<()> {
        let directory = tempfile::tempdir().map_err(|error| SmpError::io("tempdir", error))?;
        let engine = Engine::new(Paths::rooted(directory.path())?);
        engine.handle(request("conflict"))?;
        let mut changed = request("conflict");
        changed.operation = "doctor".to_owned();
        assert!(matches!(engine.handle(changed), Err(SmpError::Conflict(_))));
        Ok(())
    }

    #[test]
    fn active_records_are_not_expired() {
        let response = running_response("active");
        assert_eq!(response.state, ResultState::Running);
        assert!(response.result_handle.is_some());
    }

    #[test]
    fn result_chunks_are_binary_safe_and_bounded() -> Result<()> {
        let directory = tempfile::tempdir().map_err(|error| SmpError::io("tempdir", error))?;
        let path = directory.path().join("bytes");
        atomic_write(&path, &[0, 1, 2, 255], 0o600)?;
        let value = read_chunk(&path, 1, 2)?;
        assert_eq!(value["nextOffset"], 3);
        assert_eq!(value["dataBase64"], "AQI=");
        Ok(())
    }

    #[test]
    fn operation_catalog_is_open_without_creating_tools() {
        assert!(operation_catalog().contains(&"raw.firecracker"));
        assert!(operation_schemas().is_object());
    }
}
