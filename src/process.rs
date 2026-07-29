use crate::error::{Result, SmpError};
use crate::model::ProcessIdentity;
use crate::util::sha256_file;
use std::fs::{self, File, OpenOptions};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub fn capture(pid: i32, expected_executable: Option<&Path>) -> Result<ProcessIdentity> {
    if pid <= 0 {
        return Err(SmpError::Invalid(format!("invalid PID {pid}")));
    }
    let start_time = process_start_time(pid)?;
    let executable_path = fs::read_link(format!("/proc/{pid}/exe"))
        .map_err(|error| SmpError::io(format!("/proc/{pid}/exe"), error))?;
    let canonical_executable = fs::canonicalize(&executable_path)
        .map_err(|error| SmpError::io(executable_path.display().to_string(), error))?;
    if let Some(expected) = expected_executable {
        let expected = fs::canonicalize(expected)
            .map_err(|error| SmpError::io(expected.display().to_string(), error))?;
        if expected != canonical_executable {
            return Err(SmpError::Ambiguous(format!(
                "PID {pid} executable {} does not match {}",
                canonical_executable.display(),
                expected.display()
            )));
        }
    }
    let executable_digest = sha256_file(&canonical_executable).ok();
    let boot_id = fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_err(|error| SmpError::io("/proc/sys/kernel/random/boot_id", error))?
        .trim()
        .to_owned();
    // SAFETY: getpgid reads kernel process metadata and does not dereference user pointers.
    let process_group = unsafe { libc::getpgid(pid) };
    if process_group < 0 {
        return Err(SmpError::io(
            format!("getpgid({pid})"),
            std::io::Error::last_os_error(),
        ));
    }
    Ok(ProcessIdentity {
        pid,
        process_start_time: start_time,
        executable_path: canonical_executable,
        executable_digest,
        boot_id,
        process_group,
    })
}

pub fn verify(identity: &ProcessIdentity) -> Result<()> {
    let current = capture(identity.pid, Some(&identity.executable_path))?;
    if current.process_start_time != identity.process_start_time
        || current.boot_id != identity.boot_id
        || current.process_group != identity.process_group
        || (identity.executable_digest.is_some()
            && current.executable_digest != identity.executable_digest)
    {
        return Err(SmpError::Ambiguous(format!(
            "PID {} no longer has the recorded identity",
            identity.pid
        )));
    }
    Ok(())
}

pub fn is_running(identity: &ProcessIdentity) -> bool {
    verify(identity).is_ok()
}

pub fn process_start_time(pid: i32) -> Result<u64> {
    let path = format!("/proc/{pid}/stat");
    let value = fs::read_to_string(&path).map_err(|error| SmpError::io(path.clone(), error))?;
    let end = value
        .rfind(')')
        .ok_or_else(|| SmpError::State(format!("malformed {path}")))?;
    let fields = value[end + 1..].split_whitespace().collect::<Vec<_>>();
    let raw = fields
        .get(19)
        .ok_or_else(|| SmpError::State(format!("missing start time in {path}")))?;
    raw.parse::<u64>()
        .map_err(|_| SmpError::State(format!("invalid start time in {path}")))
}

pub fn spawn_detached(
    program: &Path,
    args: &[String],
    cwd: &Path,
    stdout_path: &Path,
    stderr_path: &Path,
) -> Result<(Child, ProcessIdentity)> {
    let stdout = open_log(stdout_path)?;
    let stderr = open_log(stderr_path)?;
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    // SAFETY: pre_exec runs after fork; setsid is async-signal-safe and captures no borrowed data.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let child = command
        .spawn()
        .map_err(|error| SmpError::io(program.display().to_string(), error))?;
    let pid = i32::try_from(child.id())
        .map_err(|_| SmpError::State("child PID exceeds i32".to_owned()))?;
    let mut last_error = None;
    for _ in 0..20 {
        match capture(pid, Some(program)) {
            Ok(identity) => return Ok((child, identity)),
            Err(error) => {
                last_error = Some(error);
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| SmpError::State("process identity unavailable".to_owned())))
}

pub fn signal(identity: &ProcessIdentity, signal: i32) -> Result<()> {
    verify(identity)?;
    // SAFETY: identity was verified immediately before signaling and kill accepts scalar values.
    let result = unsafe { libc::kill(identity.pid, signal) };
    if result == 0 {
        Ok(())
    } else {
        Err(SmpError::io(
            format!("kill({}, {signal})", identity.pid),
            std::io::Error::last_os_error(),
        ))
    }
}

pub fn wait_for_exit(identity: &ProcessIdentity, timeout: Duration) -> Result<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        match verify(identity) {
            Ok(()) if Instant::now() < deadline => thread::sleep(Duration::from_millis(100)),
            Ok(()) => return Ok(false),
            Err(SmpError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(true);
            }
            Err(SmpError::Ambiguous(_)) => return Ok(true),
            Err(error) => return Err(error),
        }
    }
}

fn open_log(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| SmpError::io(parent.display().to_string(), error))?;
    }
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|error| SmpError::io(path.display().to_string(), error))
}

pub fn verified_socket(
    identity: &ProcessIdentity,
    machine_dir: &Path,
    socket: &Path,
) -> Result<()> {
    verify(identity)?;
    let socket_parent = socket
        .parent()
        .ok_or_else(|| SmpError::Invalid("API socket has no parent".to_owned()))?;
    let expected = fs::canonicalize(machine_dir)
        .map_err(|error| SmpError::io(machine_dir.display().to_string(), error))?;
    let actual = fs::canonicalize(socket_parent)
        .map_err(|error| SmpError::io(socket_parent.display().to_string(), error))?;
    if expected != actual {
        return Err(SmpError::Ambiguous(format!(
            "API socket {} is not in machine directory {}",
            socket.display(),
            machine_dir.display()
        )));
    }
    let metadata =
        fs::metadata(socket).map_err(|error| SmpError::io(socket.display().to_string(), error))?;
    if !std::os::unix::fs::FileTypeExt::is_socket(&metadata.file_type()) {
        return Err(SmpError::Ambiguous(format!(
            "{} is not a Unix socket",
            socket.display()
        )));
    }
    Ok(())
}

pub fn current_executable() -> Result<PathBuf> {
    std::env::current_exe().map_err(|error| SmpError::io("/proc/self/exe", error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_start_time_binds_current_process() -> Result<()> {
        let pid = i32::try_from(std::process::id())
            .map_err(|_| SmpError::State("test PID overflow".to_owned()))?;
        let identity = capture(pid, None)?;
        assert!(identity.process_start_time > 0);
        verify(&identity)
    }

    #[test]
    fn stale_pid_is_rejected() {
        let identity = ProcessIdentity {
            pid: i32::MAX,
            process_start_time: 1,
            executable_path: PathBuf::from("/bin/false"),
            executable_digest: None,
            boot_id: "none".to_owned(),
            process_group: 1,
        };
        assert!(verify(&identity).is_err());
    }

    #[test]
    fn wrong_executable_is_rejected() -> Result<()> {
        let pid = i32::try_from(std::process::id())
            .map_err(|_| SmpError::State("test PID overflow".to_owned()))?;
        assert!(capture(pid, Some(Path::new("/bin/false"))).is_err());
        Ok(())
    }
}
