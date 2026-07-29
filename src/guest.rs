use crate::model::{MachineMode, MachineRecord};
use crate::state::RuntimePaths;
use crate::util::{bounded_read, file_size, set_mode, validate_guest_path};
use anyhow::{bail, Context, Result};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub fn ensure_guest_key(paths: &RuntimePaths) -> Result<PathBuf> {
    let private = paths.guest_key_path();
    let public = PathBuf::from(format!("{}.pub", private.display()));
    if private.is_file() && public.is_file() {
        return Ok(private);
    }
    let parent = private
        .parent()
        .ok_or_else(|| anyhow::anyhow!("guest key has no parent"))?;
    fs::create_dir_all(parent)?;
    let output = Command::new("ssh-keygen")
        .args([
            "-q",
            "-t",
            "ed25519",
            "-N",
            "",
            "-C",
            "smp-guest-root",
            "-f",
        ])
        .arg(&private)
        .output()
        .context("generate SMP guest SSH key")?;
    if !output.status.success() {
        bail!(
            "ssh-keygen failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    set_mode(&private, 0o600)?;
    set_mode(&public, 0o644)?;
    Ok(private)
}

pub fn create_writable_root(base: &Path, destination: &Path, mode: &MachineMode) -> Result<()> {
    if destination.exists() {
        bail!(
            "machine root disk already exists: {}",
            destination.display()
        );
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let output = Command::new("cp")
        .args([
            "--reflink=auto",
            "--sparse=always",
            "--preserve=mode,timestamps",
            "--",
        ])
        .arg(base)
        .arg(destination)
        .output()
        .context("clone immutable SMP base image")?;
    if !output.status.success() {
        bail!(
            "root disk clone failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    set_mode(destination, 0o600)?;
    match mode {
        MachineMode::Persistent | MachineMode::Disposable => Ok(()),
    }
}

pub fn create_seed(paths: &RuntimePaths, record: &MachineRecord, output: &Path) -> Result<()> {
    let key = ensure_guest_key(paths)?;
    let public = PathBuf::from(format!("{}.pub", key.display()));
    let script = paths.lib_root.join("create-seed.sh");
    if !script.is_file() {
        bail!("SMP seed builder is missing: {}", script.display());
    }
    let dns = record.network.dns_servers.join(",");
    let command = Command::new(&script)
        .arg("--output")
        .arg(output)
        .arg("--hostname")
        .arg(&record.name)
        .arg("--authorized-key-file")
        .arg(public)
        .arg("--address")
        .arg(format!(
            "{}/{}",
            record.network.guest_address, record.network.prefix_length
        ))
        .arg("--gateway")
        .arg(&record.network.gateway_address)
        .arg("--dns")
        .arg(dns)
        .arg("--mac")
        .arg(&record.network.guest_mac)
        .output()
        .with_context(|| format!("run {}", script.display()))?;
    if !command.status.success() {
        bail!(
            "seed creation failed: {}",
            String::from_utf8_lossy(&command.stderr).trim()
        );
    }
    set_mode(output, 0o600)?;
    Ok(())
}

pub fn wait_for_ssh(record: &MachineRecord, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    let mut last = String::new();
    while Instant::now() < deadline {
        match ssh_output(record, &["true".to_owned()]) {
            Ok(output) if output.status.success() => return Ok(()),
            Ok(output) => last = String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            Err(error) => last = error.to_string(),
        }
        thread::sleep(Duration::from_millis(500));
    }
    bail!("root SSH did not become ready: {last}")
}

pub fn open_shell(record: &MachineRecord) -> Result<ExitStatus> {
    let mut command = ssh_command(record);
    command.arg("-tt");
    command.arg(format!("root@{}", record.network.guest_address));
    command.status().context("open guest root shell")
}

pub fn exec_exact(record: &MachineRecord, argv: &[String], tty: bool) -> Result<ExitStatus> {
    if argv.is_empty() {
        bail!("exec requires a non-empty argv");
    }
    let mut command = ssh_command(record);
    if tty {
        command.arg("-tt");
    } else {
        command.arg("-T");
    }
    command.arg(format!("root@{}", record.network.guest_address));
    command.arg("/usr/local/libexec/smp-exec-hex");
    for value in argv {
        command.arg(hex::encode(value.as_bytes()));
    }
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("execute exact guest argv")
}

pub fn exec_capture(
    record: &MachineRecord,
    argv: &[String],
    stdin: Option<&[u8]>,
) -> Result<std::process::Output> {
    if argv.is_empty() {
        bail!("exec requires a non-empty argv");
    }
    let mut command = ssh_command(record);
    command.arg("-T");
    command.arg(format!("root@{}", record.network.guest_address));
    command.arg("/usr/local/libexec/smp-exec-hex");
    for value in argv {
        command.arg(hex::encode(value.as_bytes()));
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    let mut child = command.spawn().context("spawn guest command")?;
    if let Some(input) = stdin {
        child
            .stdin
            .as_mut()
            .expect("piped stdin")
            .write_all(input)?;
    }
    child.wait_with_output().context("wait for guest command")
}

pub fn upload(record: &MachineRecord, guest_path: &str, bytes: &[u8]) -> Result<()> {
    validate_guest_path(guest_path)?;
    let mut command = ssh_command(record);
    command.arg("-T");
    command.arg(format!("root@{}", record.network.guest_address));
    command.arg("/usr/local/libexec/smp-file-write-hex");
    command.arg(hex::encode(guest_path.as_bytes()));
    command.arg(bytes.len().to_string());
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().context("spawn guest upload")?;
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(bytes)?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!(
            "guest upload failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

pub fn download(
    record: &MachineRecord,
    guest_path: &str,
    offset: u64,
    maximum: u64,
) -> Result<Vec<u8>> {
    validate_guest_path(guest_path)?;
    let output = ssh_output(
        record,
        &[
            "/usr/local/libexec/smp-file-read-hex".to_owned(),
            hex::encode(guest_path.as_bytes()),
            offset.to_string(),
            maximum.to_string(),
        ],
    )?;
    if !output.status.success() {
        bail!(
            "guest download failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

pub fn copy_local_to_guest(record: &MachineRecord, local: &Path, guest_path: &str) -> Result<()> {
    let mut file = File::open(local).with_context(|| format!("open {}", local.display()))?;
    let length = file_size(local)?;
    if length > 256 * 1024 * 1024 {
        bail!("single-file copy exceeds the 256 MiB local CLI limit");
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.read_to_end(&mut bytes)?;
    upload(record, guest_path, &bytes)
}

pub fn copy_guest_to_local(record: &MachineRecord, guest_path: &str, local: &Path) -> Result<()> {
    validate_guest_path(guest_path)?;
    let bytes = download(record, guest_path, 0, 256 * 1024 * 1024)?;
    if let Some(parent) = local.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(local, bytes)?;
    Ok(())
}

pub fn read_local_chunk(path: &Path, offset: u64, maximum: u64) -> Result<Vec<u8>> {
    bounded_read(path, offset, maximum)
}

fn ssh_command(record: &MachineRecord) -> Command {
    let known_hosts = Path::new(&record.config_path)
        .parent()
        .unwrap_or_else(|| Path::new("/var/lib/smp"))
        .join("known_hosts");
    let mut command = Command::new("ssh");
    command
        .arg("-i")
        .arg(&record.ssh_key_path)
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("IdentitiesOnly=yes")
        .arg("-o")
        .arg("ConnectTimeout=5")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg(format!("UserKnownHostsFile={}", known_hosts.display()));
    command
}

fn ssh_output(record: &MachineRecord, argv: &[String]) -> Result<std::process::Output> {
    let mut command = ssh_command(record);
    command.arg("-T");
    command.arg(format!("root@{}", record.network.guest_address));
    if argv.len() == 1 && argv[0] == "true" {
        command.arg("true");
    } else {
        for value in argv {
            command.arg(value);
        }
    }
    command.output().context("run guest SSH command")
}

#[cfg(test)]
mod tests {
    #[test]
    fn exact_argv_encoding_has_no_shell_metacharacters() {
        let encoded = hex::encode(b"$(touch /tmp/nope); space");
        assert!(encoded.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}
