use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::NamedTempFile;

pub fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub fn validate_machine_name(name: &str) -> Result<()> {
    static NAME: OnceLock<Regex> = OnceLock::new();
    let regex = NAME.get_or_init(|| Regex::new(r"^[a-z][a-z0-9-]{0,62}$").expect("valid regex"));
    const RESERVED: &[&str] = &[
        "absent",
        "assets",
        "credentials",
        "default-state",
        "locks",
        "machines",
        "requests",
        "results",
        "run",
        "tmp",
    ];
    if !regex.is_match(name) {
        bail!("invalid machine name {name:?}: use 1-63 lowercase letters, digits, and hyphens, beginning with a letter");
    }
    if RESERVED.contains(&name) {
        bail!("reserved machine name {name:?}");
    }
    Ok(())
}

pub fn validate_guest_path(path: &str) -> Result<()> {
    let value = Path::new(path);
    if !value.is_absolute() {
        bail!("guest path must be absolute");
    }
    for component in value.components() {
        if matches!(component, Component::ParentDir | Component::CurDir | Component::Prefix(_)) {
            bail!("guest path contains a forbidden component");
        }
    }
    Ok(())
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn atomic_write_json<T: Serialize>(path: &Path, value: &T, mode: u32) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(&bytes)?;
    temporary.write_all(b"\n")?;
    temporary.as_file_mut().sync_all()?;
    set_mode(temporary.path(), mode)?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("persist {}", path.display()))?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

pub fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent", path.display()))?;
    fs::create_dir_all(parent)?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file_mut().sync_all()?;
    set_mode(temporary.path(), mode)?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("persist {}", path.display()))?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(unix)]
pub fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
pub fn set_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

pub fn process_start_time_ticks(pid: u32) -> Result<u64> {
    let value = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let close = value
        .rfind(')')
        .ok_or_else(|| anyhow!("malformed /proc/{pid}/stat"))?;
    let fields: Vec<&str> = value[close + 1..].split_whitespace().collect();
    fields
        .get(19)
        .ok_or_else(|| anyhow!("missing start time in /proc/{pid}/stat"))?
        .parse()
        .context("parse process start time")
}

pub fn process_executable(pid: u32) -> Result<PathBuf> {
    fs::read_link(format!("/proc/{pid}/exe")).context("read process executable")
}

pub fn process_matches(pid: u32, start_time_ticks: u64, executable: &Path, expected_sha256: &str) -> Result<bool> {
    if process_start_time_ticks(pid).ok() != Some(start_time_ticks) {
        return Ok(false);
    }
    let observed = match process_executable(pid) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let expected = fs::canonicalize(executable).unwrap_or_else(|_| executable.to_path_buf());
    let observed = fs::canonicalize(&observed).unwrap_or(observed);
    if observed != expected {
        return Ok(false);
    }
    Ok(sha256_file(&observed).ok().as_deref() == Some(expected_sha256))
}

pub fn command_exists(program: &str) -> bool {
    Command::new("sh")
        .args(["-c", "command -v \"$1\" >/dev/null 2>&1", "sh", program])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn run(program: impl AsRef<OsStr>, args: &[OsString]) -> Result<()> {
    let program_ref = program.as_ref();
    let status = Command::new(program_ref)
        .args(args)
        .status()
        .with_context(|| format!("run {:?}", program_ref))?;
    if !status.success() {
        bail!("{:?} exited with {status}", program_ref);
    }
    Ok(())
}

pub fn run_output(program: impl AsRef<OsStr>, args: &[OsString]) -> Result<Output> {
    let program_ref = program.as_ref();
    let output = Command::new(program_ref)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("run {:?}", program_ref))?;
    Ok(output)
}

pub fn open_append(path: &Path, mode: u32) -> Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    set_mode(path, mode)?;
    Ok(file)
}

pub fn os_strings(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
}

pub fn file_size(path: &Path) -> Result<u64> {
    Ok(fs::metadata(path)?.len())
}

pub fn bounded_read(path: &Path, offset: u64, maximum: u64) -> Result<Vec<u8>> {
    use std::io::{Seek, SeekFrom};
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut output = Vec::new();
    file.take(maximum).read_to_end(&mut output)?;
    Ok(output)
}

pub fn redact(input: &str) -> String {
    let mut output = input.to_owned();
    for marker in ["token", "secret", "password", "credential", "private_key"] {
        if output.to_ascii_lowercase().contains(marker) {
            output = "[redacted-sensitive-input]".to_owned();
            break;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_names_are_strict() {
        for valid in ["default", "a", "machine-01"] {
            validate_machine_name(valid).unwrap();
        }
        for invalid in ["", ".", "..", "/tmp/x", "A", "a_b", "-bad", "machines"] {
            assert!(validate_machine_name(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn guest_paths_reject_traversal() {
        validate_guest_path("/root/ok").unwrap();
        assert!(validate_guest_path("relative").is_err());
        assert!(validate_guest_path("/root/../etc/shadow").is_err());
    }

    #[test]
    fn digest_is_deterministic() {
        assert_eq!(sha256_bytes(b"smp"), sha256_bytes(b"smp"));
        assert_ne!(sha256_bytes(b"smp"), sha256_bytes(b"SMP"));
    }
}
