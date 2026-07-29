use crate::error::{Result, SmpError};
use crate::model::MachineRecord;
use crate::util::{canonical_json_digest, reject_symlink_components, sha256_file};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const GUEST_HELPER: &str = "/usr/local/libexec/smp";
const TRANSFER_CHUNK: usize = 48 * 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionResult {
    pub exit_code: i32,
    pub signal: Option<i32>,
    pub timed_out: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_complete: bool,
    pub stderr_complete: bool,
    pub total_stdout_bytes: u64,
    pub total_stderr_bytes: u64,
}

#[derive(Debug)]
struct ReadResult {
    retained: Vec<u8>,
    total: u64,
    complete: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FilePut {
    path: PathBuf,
    offset: u64,
    data: String,
    truncate: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileFinish {
    temporary_path: PathBuf,
    destination_path: PathBuf,
    size: u64,
    sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileGet {
    path: PathBuf,
    offset: u64,
    limit: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileChunk {
    offset: u64,
    data: String,
    eof: bool,
    total_size: u64,
    sha256: Option<String>,
}

pub fn execute(
    record: &MachineRecord,
    key: &Path,
    argv: &[String],
    stdin: Option<&[u8]>,
    timeout: Duration,
    capture_limit: u64,
    tty: bool,
) -> Result<ExecutionResult> {
    if argv.is_empty() {
        return Err(SmpError::Invalid("guest argv cannot be empty".to_owned()));
    }
    let encoded = encode(argv)?;
    let mut command = ssh_command(record, key, tty);
    command
        .arg(GUEST_HELPER)
        .arg("__guest-exec")
        .arg(encoded)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    capture(&mut command, stdin, timeout, capture_limit)
}

pub fn interactive_shell(record: &MachineRecord, key: &Path) -> Result<i32> {
    let status = ssh_command(record, key, true)
        .status()
        .map_err(|error| SmpError::io("ssh", error))?;
    Ok(exit_code(status))
}

pub fn ready(record: &MachineRecord, key: &Path, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    let argv = vec![
        "test".to_owned(),
        "-f".to_owned(),
        "/var/lib/smp-init/success.json".to_owned(),
    ];
    loop {
        let last = match execute(
            record,
            key,
            &argv,
            None,
            Duration::from_secs(10),
            64 * 1024,
            false,
        ) {
            Ok(result) if result.exit_code == 0 => return Ok(()),
            Ok(result) => format!("SSH readiness exited {}", result.exit_code),
            Err(error) => error.to_string(),
        };
        if Instant::now() >= deadline {
            return Err(SmpError::State(format!(
                "guest did not become ready: {last}"
            )));
        }
        thread::sleep(Duration::from_millis(500));
    }
}

pub fn upload(record: &MachineRecord, key: &Path, source: &Path, destination: &Path) -> Result<()> {
    if !destination.is_absolute() {
        return Err(SmpError::Invalid(
            "guest destination must be absolute".to_owned(),
        ));
    }
    let expected_digest = sha256_file(source)?;
    let expected_size = fs::metadata(source)
        .map_err(|error| SmpError::io(source.display().to_string(), error))?
        .len();
    let temporary = PathBuf::from(format!(
        "{}.smp-{}.tmp",
        destination.display(),
        Uuid::new_v4()
    ));
    let mut source_file =
        File::open(source).map_err(|error| SmpError::io(source.display().to_string(), error))?;
    let mut offset = 0_u64;
    let mut first = true;
    loop {
        let mut buffer = vec![0_u8; TRANSFER_CHUNK];
        let count = source_file
            .read(&mut buffer)
            .map_err(|error| SmpError::io(source.display().to_string(), error))?;
        buffer.truncate(count);
        if count == 0 && !first {
            break;
        }
        let request = FilePut {
            path: temporary.clone(),
            offset,
            data: base64::engine::general_purpose::STANDARD.encode(&buffer),
            truncate: first,
        };
        let _: serde_json::Value = helper_request(record, key, "__guest-file-put", &request)?;
        first = false;
        offset = offset.saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
        if count == 0 {
            break;
        }
    }
    let finish = FileFinish {
        temporary_path: temporary,
        destination_path: destination.to_path_buf(),
        size: expected_size,
        sha256: expected_digest,
    };
    let _: serde_json::Value = helper_request(record, key, "__guest-file-finish", &finish)?;
    Ok(())
}

pub fn download(
    record: &MachineRecord,
    key: &Path,
    source: &Path,
    destination: &Path,
) -> Result<()> {
    if !source.is_absolute() || !destination.is_absolute() {
        return Err(SmpError::Invalid(
            "download paths must be absolute".to_owned(),
        ));
    }
    reject_symlink_components(destination)?;
    let parent = destination
        .parent()
        .ok_or_else(|| SmpError::Invalid("download destination has no parent".to_owned()))?;
    fs::create_dir_all(parent)
        .map_err(|error| SmpError::io(parent.display().to_string(), error))?;
    let temporary = parent.join(format!(".smp-download-{}", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)
            .map_err(|error| SmpError::io(temporary.display().to_string(), error))?;
        let mut offset = 0_u64;
        let (expected_digest, total_size) = loop {
            let request = FileGet {
                path: source.to_path_buf(),
                offset,
                limit: TRANSFER_CHUNK,
            };
            let chunk: FileChunk = helper_request(record, key, "__guest-file-get", &request)?;
            if chunk.offset != offset {
                return Err(SmpError::State("guest file offset mismatch".to_owned()));
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(chunk.data)
                .map_err(|error| SmpError::Invalid(format!("invalid guest base64: {error}")))?;
            output
                .write_all(&bytes)
                .map_err(|error| SmpError::io(temporary.display().to_string(), error))?;
            offset = offset.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
            if chunk.eof {
                break (chunk.sha256, chunk.total_size);
            }
        };
        output
            .sync_all()
            .map_err(|error| SmpError::io(temporary.display().to_string(), error))?;
        if total_size != offset {
            return Err(SmpError::State("guest file size mismatch".to_owned()));
        }
        let expected = expected_digest
            .ok_or_else(|| SmpError::State("guest file digest missing".to_owned()))?;
        let actual = sha256_file(&temporary)?;
        if actual != expected {
            return Err(SmpError::State(format!(
                "guest file digest mismatch: expected {expected}, got {actual}"
            )));
        }
        fs::rename(&temporary, destination)
            .map_err(|error| SmpError::io(destination.display().to_string(), error))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn guest_entry(operation: &str, payload: Option<&str>) -> Result<i32> {
    match operation {
        "__guest-exec" => {
            let argv: Vec<String> = decode(required(payload)?)?;
            if argv.is_empty() {
                return Err(SmpError::Invalid("guest argv cannot be empty".to_owned()));
            }
            let status = Command::new(&argv[0])
                .args(&argv[1..])
                .status()
                .map_err(|error| SmpError::io(&argv[0], error))?;
            Ok(exit_code(status))
        }
        "__guest-file-put" => {
            let request: FilePut = decode(required(payload)?)?;
            guest_file_put(&request)?;
            println!("{{\"ok\":true}}");
            Ok(0)
        }
        "__guest-file-finish" => {
            let request: FileFinish = decode(required(payload)?)?;
            guest_file_finish(&request)?;
            println!("{{\"ok\":true}}");
            Ok(0)
        }
        "__guest-file-get" => {
            let request: FileGet = decode(required(payload)?)?;
            let chunk = guest_file_get(&request)?;
            serde_json::to_writer(std::io::stdout().lock(), &chunk)
                .map_err(|error| SmpError::json("<stdout>", error))?;
            println!();
            Ok(0)
        }
        _ => Err(SmpError::Invalid(format!(
            "unknown internal guest operation {operation}"
        ))),
    }
}

fn helper_request<T: Serialize, R: serde::de::DeserializeOwned>(
    record: &MachineRecord,
    key: &Path,
    operation: &str,
    request: &T,
) -> Result<R> {
    let encoded = encode(request)?;
    let mut command = ssh_command(record, key, false);
    let output = command
        .arg(GUEST_HELPER)
        .arg(operation)
        .arg(encoded)
        .output()
        .map_err(|error| SmpError::io("ssh", error))?;
    if !output.status.success() {
        return Err(SmpError::External {
            program: format!("ssh {operation}"),
            code: exit_code(output.status),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| SmpError::json(format!("guest {operation} response"), error))
}

fn guest_file_put(request: &FilePut) -> Result<()> {
    validate_guest_file_path(&request.path)?;
    reject_symlink_components(&request.path)?;
    if let Some(parent) = request.path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| SmpError::io(parent.display().to_string(), error))?;
    }
    let data = base64::engine::general_purpose::STANDARD
        .decode(&request.data)
        .map_err(|error| SmpError::Invalid(format!("invalid upload base64: {error}")))?;
    let mut options = OpenOptions::new();
    options
        .create(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW);
    if request.truncate {
        options.truncate(true);
    }
    let mut file = options
        .open(&request.path)
        .map_err(|error| SmpError::io(request.path.display().to_string(), error))?;
    file.seek(SeekFrom::Start(request.offset))
        .map_err(|error| SmpError::io(request.path.display().to_string(), error))?;
    file.write_all(&data)
        .map_err(|error| SmpError::io(request.path.display().to_string(), error))?;
    file.sync_data()
        .map_err(|error| SmpError::io(request.path.display().to_string(), error))
}

fn guest_file_finish(request: &FileFinish) -> Result<()> {
    validate_guest_file_path(&request.temporary_path)?;
    validate_guest_file_path(&request.destination_path)?;
    reject_symlink_components(&request.destination_path)?;
    let metadata = fs::metadata(&request.temporary_path)
        .map_err(|error| SmpError::io(request.temporary_path.display().to_string(), error))?;
    if metadata.len() != request.size {
        return Err(SmpError::State(format!(
            "uploaded file size mismatch: expected {}, got {}",
            request.size,
            metadata.len()
        )));
    }
    let digest = sha256_file(&request.temporary_path)?;
    if digest != request.sha256 {
        return Err(SmpError::State(format!(
            "uploaded file digest mismatch: expected {}, got {}",
            request.sha256, digest
        )));
    }
    fs::rename(&request.temporary_path, &request.destination_path)
        .map_err(|error| SmpError::io(request.destination_path.display().to_string(), error))
}

fn guest_file_get(request: &FileGet) -> Result<FileChunk> {
    validate_guest_file_path(&request.path)?;
    if request.limit == 0 || request.limit > 1024 * 1024 {
        return Err(SmpError::Invalid("invalid download chunk limit".to_owned()));
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&request.path)
        .map_err(|error| SmpError::io(request.path.display().to_string(), error))?;
    let total_size = file
        .metadata()
        .map_err(|error| SmpError::io(request.path.display().to_string(), error))?
        .len();
    file.seek(SeekFrom::Start(request.offset))
        .map_err(|error| SmpError::io(request.path.display().to_string(), error))?;
    let mut data = vec![0_u8; request.limit];
    let count = file
        .read(&mut data)
        .map_err(|error| SmpError::io(request.path.display().to_string(), error))?;
    data.truncate(count);
    let next = request
        .offset
        .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
    let eof = next >= total_size;
    Ok(FileChunk {
        offset: request.offset,
        data: base64::engine::general_purpose::STANDARD.encode(data),
        eof,
        total_size,
        sha256: if eof {
            Some(sha256_file(&request.path)?)
        } else {
            None
        },
    })
}

fn validate_guest_file_path(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        Err(SmpError::Invalid(format!(
            "unsafe guest path {}",
            path.display()
        )))
    } else {
        Ok(())
    }
}

fn capture(
    command: &mut Command,
    stdin: Option<&[u8]>,
    timeout: Duration,
    limit: u64,
) -> Result<ExecutionResult> {
    let mut child = command
        .spawn()
        .map_err(|error| SmpError::io("ssh", error))?;
    if let Some(input) = stdin
        && let Some(mut child_stdin) = child.stdin.take()
    {
        child_stdin
            .write_all(input)
            .map_err(|error| SmpError::io("ssh stdin", error))?;
    }
    drop(child.stdin.take());
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| SmpError::State("missing SSH stdout".to_owned()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| SmpError::State("missing SSH stderr".to_owned()))?;
    let budget = Arc::new(AtomicU64::new(limit));
    let stdout_budget = Arc::clone(&budget);
    let stderr_budget = Arc::clone(&budget);
    let stdout_thread = thread::spawn(move || read_bounded(stdout, stdout_budget));
    let stderr_thread = thread::spawn(move || read_bounded(stderr, stderr_budget));
    let deadline = Instant::now() + timeout;
    let (status, timed_out) = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| SmpError::io("ssh wait", error))?
        {
            break (status, false);
        }
        if Instant::now() >= deadline {
            child
                .kill()
                .map_err(|error| SmpError::io("ssh kill", error))?;
            let status = child
                .wait()
                .map_err(|error| SmpError::io("ssh wait", error))?;
            break (status, true);
        }
        thread::sleep(Duration::from_millis(20));
    };
    let stdout = stdout_thread
        .join()
        .map_err(|_| SmpError::State("SSH stdout reader panicked".to_owned()))??;
    let stderr = stderr_thread
        .join()
        .map_err(|_| SmpError::State("SSH stderr reader panicked".to_owned()))??;
    Ok(ExecutionResult {
        exit_code: exit_code(status),
        signal: exit_signal(status),
        timed_out,
        stdout: stdout.retained,
        stderr: stderr.retained,
        stdout_complete: stdout.complete,
        stderr_complete: stderr.complete,
        total_stdout_bytes: stdout.total,
        total_stderr_bytes: stderr.total,
    })
}

fn read_bounded<R: Read>(mut reader: R, budget: Arc<AtomicU64>) -> Result<ReadResult> {
    let mut retained = Vec::new();
    let mut total = 0_u64;
    let mut complete = true;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| SmpError::io("captured stream", error))?;
        if count == 0 {
            break;
        }
        total = total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
        let mut allowed = 0_usize;
        let _ = budget.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
            let count_u64 = u64::try_from(count).unwrap_or(u64::MAX);
            let take = remaining.min(count_u64);
            allowed = usize::try_from(take).unwrap_or(count);
            Some(remaining.saturating_sub(take))
        });
        retained.extend_from_slice(&buffer[..allowed]);
        if allowed < count {
            complete = false;
        }
    }
    Ok(ReadResult {
        retained,
        total,
        complete,
    })
}

fn ssh_command(record: &MachineRecord, key: &Path, tty: bool) -> Command {
    let mut command = Command::new("ssh");
    command.args([
        "-i",
        &key.display().to_string(),
        "-o",
        "BatchMode=yes",
        "-o",
        "IdentitiesOnly=yes",
        "-o",
        "StrictHostKeyChecking=accept-new",
        "-o",
        &format!(
            "UserKnownHostsFile={}",
            record.machine_directory.join("known_hosts").display()
        ),
        "-o",
        "ConnectTimeout=5",
    ]);
    if tty {
        command.arg("-tt");
    }
    command.arg(format!("root@{}", record.network.guest_address));
    command
}

fn encode<T: Serialize + ?Sized>(value: &T) -> Result<String> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| SmpError::json("<guest-payload>", error))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|error| SmpError::Invalid(format!("invalid guest payload: {error}")))?;
    serde_json::from_slice(&bytes).map_err(|error| SmpError::json("<guest-payload>", error))
}

fn required(value: Option<&str>) -> Result<&str> {
    value.ok_or_else(|| SmpError::Invalid("internal guest payload is required".to_owned()))
}

fn exit_code(status: ExitStatus) -> i32 {
    status
        .code()
        .unwrap_or_else(|| 128 + exit_signal(status).unwrap_or(0))
}

#[cfg(unix)]
fn exit_signal(status: ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

pub fn request_digest<T: Serialize>(request: &T) -> Result<String> {
    canonical_json_digest(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_argv_round_trips_metacharacters() -> Result<()> {
        let argv = vec![
            "printf".to_owned(),
            "$(touch /wrong)".to_owned(),
            "a; b".to_owned(),
            "*.rs".to_owned(),
        ];
        let encoded = encode(&argv)?;
        let decoded: Vec<String> = decode(&encoded)?;
        assert_eq!(argv, decoded);
        Ok(())
    }

    #[test]
    fn guest_path_traversal_is_rejected() {
        assert!(validate_guest_file_path(Path::new("/tmp/../etc/passwd")).is_err());
        assert!(validate_guest_file_path(Path::new("relative")).is_err());
        assert!(validate_guest_file_path(Path::new("/root/file")).is_ok());
    }

    #[test]
    fn file_payload_digest_is_deterministic() -> Result<()> {
        let request = FileGet {
            path: "/root/file".into(),
            offset: 7,
            limit: 1024,
        };
        assert_eq!(request_digest(&request)?, request_digest(&request)?);
        Ok(())
    }
}
