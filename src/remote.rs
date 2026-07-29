use crate::core::{self, CreateOptions};
use crate::doctor;
use crate::guest;
use crate::model::{
    GoRequest, GoResponse, MachineMode, OperationSchema, ProcessIdentity, PublishedPort,
    RequestRecord, ResultRecord, TypedError, VirtioTransport, BUILD_COMMIT, CAPTURE_OUTPUT_LIMIT,
    FIRECRACKER_VERSION, INLINE_OUTPUT_LIMIT, KERNEL_VERSION, MAX_TIMEOUT_SECONDS,
    REQUEST_RETENTION_SECONDS, REQUEST_SCHEMA_VERSION, RESPONSE_SCHEMA_VERSION,
    RESULT_RETENTION_SECONDS, SMP_VERSION,
};
use crate::state::{
    list_machines, load_request, load_result, save_request, save_result, RuntimePaths,
};
use crate::util::{
    atomic_write_json, bounded_read, now_unix_ms, process_matches, process_start_time_ticks,
    redact, sha256_bytes, sha256_file,
};
use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DetachedOperation {
    schema_version: u32,
    request_id: String,
    operation: String,
    machine: Option<String>,
    argv: Vec<String>,
    timeout_seconds: u64,
    capture_limit_bytes: u64,
}

pub fn validate_request(request: &GoRequest) -> Result<()> {
    if request.schema_version != REQUEST_SCHEMA_VERSION {
        bail!(
            "unsupported request schema {}; expected {}",
            request.schema_version,
            REQUEST_SCHEMA_VERSION
        );
    }
    validate_id(&request.request_id)?;
    if request.operation.trim().is_empty() || request.operation.len() > 128 {
        bail!("operation must be a non-empty string of at most 128 bytes");
    }
    let timeout = request.timeout_seconds.unwrap_or(300);
    if timeout == 0 || timeout > MAX_TIMEOUT_SECONDS {
        bail!("timeoutSeconds must be between 1 and {MAX_TIMEOUT_SECONDS}");
    }
    let output_limit = request.output_limit_bytes.unwrap_or(INLINE_OUTPUT_LIMIT);
    if output_limit == 0 || output_limit > CAPTURE_OUTPUT_LIMIT {
        bail!("outputLimitBytes must be between 1 and {CAPTURE_OUTPUT_LIMIT}");
    }
    if request.argv.len() > 4096 || request.argv.iter().any(|value| value.len() > 1_048_576) {
        bail!("argv exceeds SMP request limits");
    }
    Ok(())
}

pub fn request_digest(request: &GoRequest) -> Result<String> {
    validate_request(request)?;
    let bytes = serde_json::to_vec(request)?;
    Ok(sha256_bytes(&bytes))
}

pub fn handle_go(paths: &RuntimePaths, request: GoRequest) -> GoResponse {
    match handle_go_inner(paths, request.clone()) {
        Ok(response) => response,
        Err(error) => GoResponse::failed(
            request.request_id,
            TypedError::new("SMP_OPERATION_FAILED", redact(&error.to_string())),
        ),
    }
}

fn handle_go_inner(paths: &RuntimePaths, request: GoRequest) -> Result<GoResponse> {
    validate_request(&request)?;
    paths.ensure()?;
    let digest = request_digest(&request)?;
    let lock_path = paths
        .run_root
        .join("request-locks")
        .join(format!("{}.lock", request.request_id));
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.lock_exclusive()?;

    if let Some(existing) = load_request(paths, &request.request_id)? {
        if existing.request_digest != digest {
            return Ok(GoResponse::failed(
                request.request_id,
                TypedError::new(
                    "REQUEST_ID_CONFLICT",
                    "requestId was already used with another normalized request",
                ),
            ));
        }
        if let Some(response) = existing.terminal_response {
            return Ok(response);
        }
        if let Some(handle) = existing.result_handle {
            return result_status_response(paths, &request.request_id, &handle);
        }
        return Ok(GoResponse::failed(
            request.request_id,
            TypedError::new(
                "REQUEST_STATE_AMBIGUOUS",
                "existing request has no terminal response or result handle",
            ),
        ));
    }

    let now = now_unix_ms();
    let mut record = RequestRecord {
        schema_version: 1,
        request_id: request.request_id.clone(),
        request_digest: digest,
        operation: redact(&request.operation),
        machine: request.machine.clone(),
        state: "running".to_owned(),
        result_handle: None,
        process: None,
        created_at_unix_ms: now,
        updated_at_unix_ms: now,
        terminal_response: None,
    };
    save_request(paths, &record)?;

    if request.detach {
        let response = start_detached(paths, &request, &mut record)?;
        save_request(paths, &record)?;
        return Ok(response);
    }

    let response = dispatch(paths, &request)?;
    record.state = response.state.clone();
    record.updated_at_unix_ms = now_unix_ms();
    record.result_handle = response.result_handle.clone();
    record.terminal_response = if response.state == "running" {
        None
    } else {
        Some(response.clone())
    };
    save_request(paths, &record)?;
    Ok(response)
}

fn dispatch(paths: &RuntimePaths, request: &GoRequest) -> Result<GoResponse> {
    match request.operation.as_str() {
        "describe" => Ok(GoResponse::completed(
            &request.request_id,
            describe(
                paths,
                option_bool(request, "includeMachines").unwrap_or(false),
            )?,
        )),
        "doctor" => Ok(GoResponse::completed(
            &request.request_id,
            serde_json::to_value(doctor::run_doctor(
                paths,
                option_bool(request, "fix").unwrap_or(false),
            )?)?,
        )),
        "machine.create" => {
            let options = create_options(request)?;
            Ok(GoResponse::completed(
                &request.request_id,
                serde_json::to_value(core::create(paths, &options)?)?,
            ))
        }
        "machine.start" => Ok(GoResponse::completed(
            &request.request_id,
            serde_json::to_value(core::start(paths, machine(request)?, false)?)?,
        )),
        "machine.wait" => Ok(GoResponse::completed(
            &request.request_id,
            serde_json::to_value(core::wait(
                paths,
                machine(request)?,
                Duration::from_secs(request.timeout_seconds.unwrap_or(300)),
            )?)?,
        )),
        "machine.status" | "machine.inspect" => Ok(GoResponse::completed(
            &request.request_id,
            serde_json::to_value(core::status(paths, machine(request)?)?)?,
        )),
        "machine.stop" => Ok(GoResponse::completed(
            &request.request_id,
            serde_json::to_value(core::stop(paths, machine(request)?)?)?,
        )),
        "machine.kill" => Ok(GoResponse::completed(
            &request.request_id,
            serde_json::to_value(core::kill(paths, machine(request)?)?)?,
        )),
        "machine.reboot" => {
            let (old, current) = core::reboot(paths, machine(request)?)?;
            Ok(GoResponse::completed(
                &request.request_id,
                json!({"oldProcess": old, "machine": current}),
            ))
        }
        "machine.destroy" => {
            core::destroy(
                paths,
                machine(request)?,
                option_bool(request, "force").unwrap_or(false),
            )?;
            Ok(GoResponse::completed(
                &request.request_id,
                json!({"destroyed": true}),
            ))
        }
        "machine.reconcile" => Ok(GoResponse::completed(
            &request.request_id,
            serde_json::to_value(core::reconcile(paths, machine(request)?)?)?,
        )),
        "exec" => execute_guest(paths, request),
        "file.upload" => file_upload(paths, request),
        "file.download" => file_download(paths, request),
        "logs.read" => logs_read(paths, request),
        "raw.smp" => raw_smp(paths, request),
        "raw.firecracker" => raw_firecracker(paths, request),
        "result.get" => result_get(paths, request),
        "result.read" => result_read(paths, request),
        "result.wait" => result_wait(paths, request),
        "result.cancel" => result_cancel(paths, request),
        other => {
            bail!("unknown SMP operation {other:?}; call describe for the live operation catalog")
        }
    }
}

pub fn describe(paths: &RuntimePaths, include_machines: bool) -> Result<Value> {
    let manifest = crate::assets::describe_manifest(paths);
    let machines = if include_machines {
        serde_json::to_value(list_machines(paths)?)?
    } else {
        Value::Array(Vec::new())
    };
    Ok(json!({
        "smpVersion": SMP_VERSION,
        "buildCommit": BUILD_COMMIT,
        "requestSchemaVersion": REQUEST_SCHEMA_VERSION,
        "responseSchemaVersion": RESPONSE_SCHEMA_VERSION,
        "operations": operation_catalog(),
        "firecracker": {"version": FIRECRACKER_VERSION, "identity": manifest.get("firecracker").cloned().unwrap_or(Value::Null)},
        "kernel": {"version": KERNEL_VERSION, "identity": manifest.get("kernel").cloned().unwrap_or(Value::Null)},
        "rootfs": manifest.get("rootfs").cloned().unwrap_or(Value::Null),
        "hostArchitecture": std::env::consts::ARCH,
        "serviceIdentity": {"pid": std::process::id(), "startTimeTicks": process_start_time_ticks(std::process::id()).ok()},
        "transport": {"mode": "mcp-http-loopback", "endpoint": "http://127.0.0.1:7745/mcp"},
        "limits": {
            "inlineOutputBytes": INLINE_OUTPUT_LIMIT,
            "capturedOutputBytes": CAPTURE_OUTPUT_LIMIT,
            "maximumTimeoutSeconds": MAX_TIMEOUT_SECONDS,
            "requestRetentionSeconds": REQUEST_RETENTION_SECONDS,
            "resultRetentionSeconds": RESULT_RETENTION_SECONDS
        },
        "machines": machines
    }))
}

pub fn operation_catalog() -> Vec<OperationSchema> {
    let names = [
        ("describe", "Return live SMP capabilities and identities"),
        ("doctor", "Inspect or fix ordinary SMP host prerequisites"),
        (
            "machine.create",
            "Create persistent or disposable writable machine state",
        ),
        ("machine.start", "Start the selected Firecracker microVM"),
        (
            "machine.wait",
            "Wait for verified Firecracker, initialization, and root SSH",
        ),
        ("machine.status", "Return current reconciled machine state"),
        (
            "machine.inspect",
            "Return the complete selected machine record",
        ),
        ("machine.stop", "Gracefully stop and preserve machine state"),
        (
            "machine.kill",
            "Kill only the verified selected Firecracker process",
        ),
        (
            "machine.reboot",
            "Host-mediate a new Firecracker process over preserved state",
        ),
        (
            "machine.destroy",
            "Explicitly remove the selected machine writable state",
        ),
        (
            "machine.reconcile",
            "Reconstruct only unambiguous runtime state",
        ),
        ("exec", "Execute an exact argv as guest UID 0"),
        (
            "file.upload",
            "Write bounded content to an absolute guest path",
        ),
        (
            "file.download",
            "Read a bounded chunk from an absolute guest path",
        ),
        ("logs.read", "Read a bounded serial-log chunk"),
        ("raw.smp", "Execute only the SMP executable with exact argv"),
        (
            "raw.firecracker",
            "Call only the selected machine verified Firecracker API socket",
        ),
        ("result.get", "Return retained result metadata"),
        ("result.read", "Read retained stdout or stderr by offset"),
        ("result.wait", "Wait for a retained operation"),
        (
            "result.cancel",
            "Cancel a verified retained operation process",
        ),
    ];
    names
        .into_iter()
        .map(|(name, summary)| OperationSchema {
            name: name.to_owned(),
            summary: summary.to_owned(),
            arguments: json!({"machine": "string?", "argv": "string[]?", "options": "object?"}),
        })
        .collect()
}

fn execute_guest(paths: &RuntimePaths, request: &GoRequest) -> Result<GoResponse> {
    let record = core::wait(
        paths,
        machine(request)?,
        Duration::from_secs(request.timeout_seconds.unwrap_or(300)),
    )?;
    let output = guest::exec_capture(
        &record,
        &request.argv,
        request.stdin.as_deref().map(str::as_bytes),
    )?;
    capture_output(paths, request, output)
}

fn file_upload(paths: &RuntimePaths, request: &GoRequest) -> Result<GoResponse> {
    let record = core::wait(
        paths,
        machine(request)?,
        Duration::from_secs(request.timeout_seconds.unwrap_or(300)),
    )?;
    let path = option_string(request, "path")?;
    let bytes = match (
        request.options.get("contentUtf8"),
        request.options.get("contentBase64"),
    ) {
        (Some(Value::String(value)), None) => value.as_bytes().to_vec(),
        (None, Some(Value::String(value))) => {
            BASE64.decode(value).context("decode contentBase64")?
        }
        _ => bail!(
            "file.upload requires exactly one of options.contentUtf8 or options.contentBase64"
        ),
    };
    if bytes.len() as u64 > CAPTURE_OUTPUT_LIMIT {
        bail!("upload exceeds the advertised captured-output limit");
    }
    guest::upload(&record, &path, &bytes)?;
    Ok(GoResponse::completed(
        &request.request_id,
        json!({"path": path, "bytes": bytes.len(), "sha256": sha256_bytes(&bytes)}),
    ))
}

fn file_download(paths: &RuntimePaths, request: &GoRequest) -> Result<GoResponse> {
    let record = core::wait(
        paths,
        machine(request)?,
        Duration::from_secs(request.timeout_seconds.unwrap_or(300)),
    )?;
    let path = option_string(request, "path")?;
    let offset = option_u64(request, "offset").unwrap_or(0);
    let maximum = option_u64(request, "maximumBytes")
        .unwrap_or(65_536)
        .min(1_048_576);
    let bytes = guest::download(&record, &path, offset, maximum)?;
    Ok(GoResponse::completed(
        &request.request_id,
        json!({
            "path": path,
            "offset": offset,
            "bytes": bytes.len(),
            "contentBase64": BASE64.encode(&bytes),
            "sha256": sha256_bytes(&bytes)
        }),
    ))
}

fn logs_read(paths: &RuntimePaths, request: &GoRequest) -> Result<GoResponse> {
    let record = core::status(paths, machine(request)?)?;
    let offset = option_u64(request, "offset").unwrap_or(0);
    let maximum = option_u64(request, "maximumBytes")
        .unwrap_or(65_536)
        .min(1_048_576);
    let bytes = bounded_read(Path::new(&record.serial_log_path), offset, maximum)?;
    Ok(GoResponse::completed(
        &request.request_id,
        json!({
            "offset": offset,
            "bytes": bytes.len(),
            "contentBase64": BASE64.encode(&bytes)
        }),
    ))
}

fn raw_smp(paths: &RuntimePaths, request: &GoRequest) -> Result<GoResponse> {
    if request.argv.is_empty() {
        bail!("raw.smp requires argv");
    }
    let executable = std::env::current_exe()?;
    let mut command = Command::new(executable);
    command
        .args(&request.argv)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_path_environment(&mut command, paths);
    let output = command.output()?;
    capture_output(paths, request, output)
}

fn raw_firecracker(paths: &RuntimePaths, request: &GoRequest) -> Result<GoResponse> {
    let method = option_string(request, "method")?;
    let path = option_string(request, "path")?;
    let body = match request.options.get("bodyBase64") {
        Some(Value::String(value)) => BASE64.decode(value)?,
        None => request
            .options
            .get("bodyUtf8")
            .and_then(Value::as_str)
            .unwrap_or("")
            .as_bytes()
            .to_vec(),
        _ => bail!("bodyBase64 must be a string"),
    };
    let headers = request
        .options
        .get("headers")
        .and_then(Value::as_object)
        .map(|values| {
            values
                .iter()
                .map(|(name, value)| {
                    value
                        .as_str()
                        .map(|value| (name.clone(), value.to_owned()))
                        .ok_or_else(|| anyhow::anyhow!("header values must be strings"))
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    let (status, response) = core::api(paths, machine(request)?, &method, &path, &headers, &body)?;
    Ok(GoResponse::completed(
        &request.request_id,
        json!({
            "httpStatus": status,
            "bodyBase64": BASE64.encode(response)
        }),
    ))
}

fn capture_output(paths: &RuntimePaths, request: &GoRequest, output: Output) -> Result<GoResponse> {
    let inline_limit = request
        .output_limit_bytes
        .unwrap_or(INLINE_OUTPUT_LIMIT)
        .min(INLINE_OUTPUT_LIMIT) as usize;
    let total = output.stdout.len().saturating_add(output.stderr.len());
    let exit_code = output.status.code();
    if total <= inline_limit {
        return Ok(GoResponse {
            schema_version: RESPONSE_SCHEMA_VERSION,
            request_id: request.request_id.clone(),
            state: "completed".to_owned(),
            exit_code,
            timed_out: false,
            cancelled: false,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            stdout_complete: true,
            stderr_complete: true,
            capture_exhausted: false,
            result_handle: None,
            machine_state: None,
            error: None,
            data: Value::Null,
        });
    }
    let handle = Uuid::new_v4().simple().to_string();
    let directory = paths.result_dir(&handle)?;
    fs::create_dir_all(&directory)?;
    let capture_limit = CAPTURE_OUTPUT_LIMIT as usize;
    let stdout_len = output.stdout.len().min(capture_limit);
    let remaining = capture_limit.saturating_sub(stdout_len);
    let stderr_len = output.stderr.len().min(remaining);
    fs::write(directory.join("stdout"), &output.stdout[..stdout_len])?;
    fs::write(directory.join("stderr"), &output.stderr[..stderr_len])?;
    let result = ResultRecord {
        schema_version: 1,
        handle: handle.clone(),
        request_id: request.request_id.clone(),
        state: "completed".to_owned(),
        process: None,
        stdout_path: directory.join("stdout").to_string_lossy().into_owned(),
        stderr_path: directory.join("stderr").to_string_lossy().into_owned(),
        stdout_bytes: stdout_len as u64,
        stderr_bytes: stderr_len as u64,
        stdout_complete: stdout_len == output.stdout.len(),
        stderr_complete: stderr_len == output.stderr.len(),
        capture_exhausted: stdout_len != output.stdout.len() || stderr_len != output.stderr.len(),
        exit_code,
        cancelled: false,
        created_at_unix_ms: now_unix_ms(),
        updated_at_unix_ms: now_unix_ms(),
        error: None,
    };
    save_result(paths, &result)?;
    Ok(response_from_result(&request.request_id, &result))
}

fn start_detached(
    paths: &RuntimePaths,
    request: &GoRequest,
    request_record: &mut RequestRecord,
) -> Result<GoResponse> {
    if request.stdin.is_some() {
        bail!("detached operations do not retain stdin; write input to a guest file first");
    }
    if !matches!(request.operation.as_str(), "exec" | "raw.smp") {
        bail!("detach is supported only for exec and raw.smp");
    }
    let handle = Uuid::new_v4().simple().to_string();
    let directory = paths.result_dir(&handle)?;
    fs::create_dir_all(&directory)?;
    let operation = DetachedOperation {
        schema_version: 1,
        request_id: request.request_id.clone(),
        operation: request.operation.clone(),
        machine: request.machine.clone(),
        argv: request.argv.clone(),
        timeout_seconds: request.timeout_seconds.unwrap_or(300),
        capture_limit_bytes: request
            .output_limit_bytes
            .unwrap_or(CAPTURE_OUTPUT_LIMIT)
            .min(CAPTURE_OUTPUT_LIMIT),
    };
    atomic_write_json(&directory.join("operation.json"), &operation, 0o600)?;
    fs::write(directory.join("stdout"), b"")?;
    fs::write(directory.join("stderr"), b"")?;
    let now = now_unix_ms();
    let mut result = ResultRecord {
        schema_version: 1,
        handle: handle.clone(),
        request_id: request.request_id.clone(),
        state: "running".to_owned(),
        process: None,
        stdout_path: directory.join("stdout").to_string_lossy().into_owned(),
        stderr_path: directory.join("stderr").to_string_lossy().into_owned(),
        stdout_bytes: 0,
        stderr_bytes: 0,
        stdout_complete: false,
        stderr_complete: false,
        capture_exhausted: false,
        exit_code: None,
        cancelled: false,
        created_at_unix_ms: now,
        updated_at_unix_ms: now,
        error: None,
    };
    save_result(paths, &result)?;

    let executable = std::env::current_exe()?;
    let executable_sha256 = sha256_file(&executable)?;
    let worker_log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("worker.log"))?;
    let worker_error = worker_log.try_clone()?;
    let mut command = Command::new(&executable);
    command
        .arg("__detached-worker")
        .arg("--handle")
        .arg(&handle)
        .stdin(Stdio::null())
        .stdout(Stdio::from(worker_log))
        .stderr(Stdio::from(worker_error));
    apply_path_environment(&mut command, paths);
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn()?;
    let identity = ProcessIdentity {
        pid: child.id(),
        start_time_ticks: process_start_time_ticks(child.id())?,
        executable: executable.to_string_lossy().into_owned(),
        executable_sha256,
    };
    result.process = Some(identity.clone());
    save_result(paths, &result)?;
    request_record.state = "running".to_owned();
    request_record.result_handle = Some(handle.clone());
    request_record.process = Some(identity);
    request_record.updated_at_unix_ms = now_unix_ms();
    Ok(response_from_result(&request.request_id, &result))
}

pub fn run_detached_worker(paths: &RuntimePaths, handle: &str) -> Result<()> {
    validate_id(handle)?;
    let directory = paths.result_dir(handle)?;
    let operation: DetachedOperation = crate::util::read_json(&directory.join("operation.json"))?;
    let mut result = load_result(paths, handle)?;
    let outcome = match operation.operation.as_str() {
        "exec" => {
            let machine = operation
                .machine
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("detached exec requires machine"))?;
            let record = core::wait(
                paths,
                machine,
                Duration::from_secs(operation.timeout_seconds),
            )?;
            guest::exec_capture(&record, &operation.argv, None)
        }
        "raw.smp" => {
            let executable = std::env::current_exe()?;
            let mut command = Command::new(executable);
            command
                .args(&operation.argv)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            apply_path_environment(&mut command, paths);
            command.output().context("run detached raw.smp")
        }
        _ => bail!("unsupported detached operation"),
    };
    match outcome {
        Ok(output) => {
            let limit = operation.capture_limit_bytes.min(CAPTURE_OUTPUT_LIMIT) as usize;
            let stdout_len = output.stdout.len().min(limit);
            let stderr_len = output.stderr.len().min(limit.saturating_sub(stdout_len));
            fs::write(&result.stdout_path, &output.stdout[..stdout_len])?;
            fs::write(&result.stderr_path, &output.stderr[..stderr_len])?;
            result.stdout_bytes = stdout_len as u64;
            result.stderr_bytes = stderr_len as u64;
            result.stdout_complete = stdout_len == output.stdout.len();
            result.stderr_complete = stderr_len == output.stderr.len();
            result.capture_exhausted = !result.stdout_complete || !result.stderr_complete;
            result.exit_code = output.status.code();
            result.state = "completed".to_owned();
            result.process = None;
        }
        Err(error) => {
            result.state = "failed".to_owned();
            result.process = None;
            result.error = Some(TypedError::new(
                "DETACHED_OPERATION_FAILED",
                redact(&error.to_string()),
            ));
        }
    }
    result.updated_at_unix_ms = now_unix_ms();
    save_result(paths, &result)?;
    let _ = fs::remove_file(directory.join("operation.json"));
    if let Some(mut request) = load_request(paths, &operation.request_id)? {
        request.state = result.state.clone();
        request.process = None;
        request.updated_at_unix_ms = now_unix_ms();
        request.terminal_response = Some(response_from_result(&operation.request_id, &result));
        save_request(paths, &request)?;
    }
    Ok(())
}

fn result_get(paths: &RuntimePaths, request: &GoRequest) -> Result<GoResponse> {
    let handle = option_string(request, "handle")?;
    result_status_response(paths, &request.request_id, &handle)
}

fn result_status_response(
    paths: &RuntimePaths,
    request_id: &str,
    handle: &str,
) -> Result<GoResponse> {
    let mut result = load_result(paths, handle)?;
    refresh_result(paths, &mut result)?;
    Ok(response_from_result(request_id, &result))
}

fn refresh_result(paths: &RuntimePaths, result: &mut ResultRecord) -> Result<()> {
    if result.state != "running" {
        return Ok(());
    }
    let Some(process) = &result.process else {
        result.state = "failed".to_owned();
        result.error = Some(TypedError::new(
            "RESULT_PROCESS_MISSING",
            "running result has no process identity",
        ));
        result.updated_at_unix_ms = now_unix_ms();
        return save_result(paths, result);
    };
    if !process_matches(
        process.pid,
        process.start_time_ticks,
        Path::new(&process.executable),
        &process.executable_sha256,
    )? {
        result.state = "failed".to_owned();
        result.process = None;
        result.error = Some(TypedError::new(
            "RESULT_PROCESS_LOST",
            "detached process exited without terminal metadata",
        ));
        result.updated_at_unix_ms = now_unix_ms();
        save_result(paths, result)?;
    }
    Ok(())
}

fn result_read(paths: &RuntimePaths, request: &GoRequest) -> Result<GoResponse> {
    let handle = option_string(request, "handle")?;
    let stream = request
        .options
        .get("stream")
        .and_then(Value::as_str)
        .unwrap_or("stdout");
    let offset = option_u64(request, "offset").unwrap_or(0);
    let maximum = option_u64(request, "maximumBytes")
        .unwrap_or(65_536)
        .min(1_048_576);
    let mut result = load_result(paths, &handle)?;
    refresh_result(paths, &mut result)?;
    let path = match stream {
        "stdout" => &result.stdout_path,
        "stderr" => &result.stderr_path,
        _ => bail!("stream must be stdout or stderr"),
    };
    let bytes = bounded_read(Path::new(path), offset, maximum)?;
    Ok(GoResponse::completed(
        &request.request_id,
        json!({
            "handle": handle,
            "stream": stream,
            "offset": offset,
            "bytes": bytes.len(),
            "contentBase64": BASE64.encode(bytes),
            "state": result.state
        }),
    ))
}

fn result_wait(paths: &RuntimePaths, request: &GoRequest) -> Result<GoResponse> {
    let handle = option_string(request, "handle")?;
    let deadline = Instant::now() + Duration::from_secs(request.timeout_seconds.unwrap_or(300));
    loop {
        let mut result = load_result(paths, &handle)?;
        refresh_result(paths, &mut result)?;
        if result.state != "running" || Instant::now() >= deadline {
            return Ok(response_from_result(&request.request_id, &result));
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn result_cancel(paths: &RuntimePaths, request: &GoRequest) -> Result<GoResponse> {
    let handle = option_string(request, "handle")?;
    let mut result = load_result(paths, &handle)?;
    refresh_result(paths, &mut result)?;
    if result.state == "running" {
        let process = result
            .process
            .clone()
            .ok_or_else(|| anyhow::anyhow!("running result has no process"))?;
        if !process_matches(
            process.pid,
            process.start_time_ticks,
            Path::new(&process.executable),
            &process.executable_sha256,
        )? {
            bail!("refusing to cancel stale process identity");
        }
        let status = unsafe { libc::kill(-(process.pid as i32), libc::SIGTERM) };
        if status != 0 {
            return Err(std::io::Error::last_os_error()).context("cancel detached process group");
        }
        result.state = "cancelled".to_owned();
        result.cancelled = true;
        result.process = None;
        result.updated_at_unix_ms = now_unix_ms();
        save_result(paths, &result)?;
    }
    Ok(response_from_result(&request.request_id, &result))
}

fn response_from_result(request_id: &str, result: &ResultRecord) -> GoResponse {
    GoResponse {
        schema_version: RESPONSE_SCHEMA_VERSION,
        request_id: request_id.to_owned(),
        state: result.state.clone(),
        exit_code: result.exit_code,
        timed_out: false,
        cancelled: result.cancelled,
        stdout: String::new(),
        stderr: String::new(),
        stdout_complete: result.stdout_complete,
        stderr_complete: result.stderr_complete,
        capture_exhausted: result.capture_exhausted,
        result_handle: Some(result.handle.clone()),
        machine_state: None,
        error: result.error.clone(),
        data: json!({"stdoutBytes": result.stdout_bytes, "stderrBytes": result.stderr_bytes}),
    }
}

fn create_options(request: &GoRequest) -> Result<CreateOptions> {
    let mut options = CreateOptions::default();
    options.name = machine(request)?.to_owned();
    options.mode = match request
        .options
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("persistent")
    {
        "persistent" => MachineMode::Persistent,
        "disposable" => MachineMode::Disposable,
        value => bail!("invalid machine mode {value}"),
    };
    options.transport = match request
        .options
        .get("transport")
        .and_then(Value::as_str)
        .unwrap_or("pci")
    {
        "pci" => VirtioTransport::Pci,
        "mmio" => VirtioTransport::Mmio,
        value => bail!("invalid VirtIO transport {value}"),
    };
    options.vcpu_count = option_u64(request, "vcpuCount").unwrap_or(2).try_into()?;
    options.memory_mib = option_u64(request, "memoryMiB")
        .unwrap_or(2048)
        .try_into()?;
    options.offline = option_bool(request, "offline").unwrap_or(false);
    options.rootfs = option_path(request, "rootfs");
    options.kernel = option_path(request, "kernel");
    options.firecracker = option_path(request, "firecracker");
    options.boot_args = request
        .options
        .get("bootArgs")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if let Some(Value::Array(ports)) = request.options.get("publishedPorts") {
        for value in ports {
            let object = value
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("publishedPorts entries must be objects"))?;
            options.published_ports.push(PublishedPort {
                protocol: object
                    .get("protocol")
                    .and_then(Value::as_str)
                    .unwrap_or("tcp")
                    .to_owned(),
                host_port: object
                    .get("hostPort")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| anyhow::anyhow!("hostPort is required"))?
                    .try_into()?,
                guest_port: object
                    .get("guestPort")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| anyhow::anyhow!("guestPort is required"))?
                    .try_into()?,
            });
        }
    }
    Ok(options)
}

fn machine(request: &GoRequest) -> Result<&str> {
    request
        .machine
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("operation requires machine"))
}

fn option_string(request: &GoRequest, name: &str) -> Result<String> {
    request
        .options
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("options.{name} must be a string"))
}

fn option_bool(request: &GoRequest, name: &str) -> Option<bool> {
    request.options.get(name).and_then(Value::as_bool)
}

fn option_u64(request: &GoRequest, name: &str) -> Option<u64> {
    request.options.get(name).and_then(Value::as_u64)
}

fn option_path(request: &GoRequest, name: &str) -> Option<PathBuf> {
    request
        .options
        .get(name)
        .and_then(Value::as_str)
        .map(PathBuf::from)
}

fn validate_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("invalid request or result identifier");
    }
    Ok(())
}

fn apply_path_environment(command: &mut Command, paths: &RuntimePaths) {
    command
        .env("SMP_ETC_ROOT", &paths.etc_root)
        .env("SMP_STATE_ROOT", &paths.state_root)
        .env("SMP_RUN_ROOT", &paths.run_root)
        .env("SMP_LIB_ROOT", &paths.lib_root);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn request() -> GoRequest {
        GoRequest {
            schema_version: 1,
            request_id: "request-1".to_owned(),
            operation: "describe".to_owned(),
            machine: None,
            argv: Vec::new(),
            stdin: None,
            timeout_seconds: Some(300),
            output_limit_bytes: Some(1024),
            detach: false,
            options: BTreeMap::new(),
        }
    }

    #[test]
    fn request_digest_is_deterministic() {
        assert_eq!(
            request_digest(&request()).unwrap(),
            request_digest(&request()).unwrap()
        );
    }

    #[test]
    fn request_schema_is_strict() {
        let mut value = request();
        value.schema_version = 2;
        assert!(validate_request(&value).is_err());
    }

    #[test]
    fn catalog_has_one_describe_and_required_operations() {
        let names: Vec<String> = operation_catalog()
            .into_iter()
            .map(|value| value.name)
            .collect();
        assert_eq!(
            names
                .iter()
                .filter(|name| name.as_str() == "describe")
                .count(),
            1
        );
        assert!(names.contains(&"raw.firecracker".to_owned()));
        assert!(names.contains(&"result.cancel".to_owned()));
    }
}
