use crate::error::{Result, SmpError};
use crate::paths::Paths;
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::Command;

const KVM_GET_API_VERSION: libc::c_ulong = 0xAE00;
const EXPECTED_KVM_API_VERSION: i32 = 12;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    pub healthy: bool,
    pub architecture: String,
    pub kernel: String,
    pub checks: Vec<Check>,
}

pub fn inspect(paths: &Paths) -> DoctorReport {
    let architecture = std::env::consts::ARCH.to_owned();
    let kernel = command_text("uname", &["-sr"]).unwrap_or_else(|error| error.to_string());
    let mut checks = Vec::new();
    checks.push(check(
        "architecture",
        architecture == "x86_64",
        architecture.clone(),
    ));
    match kvm_api_version(Path::new("/dev/kvm")) {
        Ok(version) => checks.push(check(
            "kvm",
            version == EXPECTED_KVM_API_VERSION,
            format!("KVM API version {version}"),
        )),
        Err(error) => checks.push(check("kvm", false, error.to_string())),
    }
    checks.push(path_check("/dev/net/tun", "tun"));
    for command in [
        "firecracker",
        "nft",
        "ip",
        "ssh",
        "ssh-keygen",
        "mkfs.ext4",
        "blkid",
    ] {
        let found = command_exists(command);
        checks.push(check(
            command,
            found,
            if found {
                "available".to_owned()
            } else {
                "not found in PATH".to_owned()
            },
        ));
    }
    checks.push(check(
        "ip-forwarding",
        fs::read_to_string("/proc/sys/net/ipv4/ip_forward").is_ok_and(|value| value.trim() == "1"),
        fs::read_to_string("/proc/sys/net/ipv4/ip_forward")
            .map(|value| value.trim().to_owned())
            .unwrap_or_else(|error| error.to_string()),
    ));
    checks.push(check(
        "state-root",
        paths.state.is_absolute(),
        paths.state.display().to_string(),
    ));
    let healthy = checks.iter().all(|item| item.ok);
    DoctorReport {
        healthy,
        architecture,
        kernel,
        checks,
    }
}

pub fn fix(paths: &Paths) -> Result<Vec<String>> {
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } != 0 {
        return Err(SmpError::Invalid(
            "doctor --fix requires root and never prompts for sudo".to_owned(),
        ));
    }
    let mut changes = Vec::new();
    paths.ensure_layout()?;
    changes.push("ensured SMP-owned directory layout and permissions".to_owned());
    let forwarding = fs::read_to_string("/proc/sys/net/ipv4/ip_forward")
        .map_err(|error| SmpError::io("/proc/sys/net/ipv4/ip_forward", error))?;
    if forwarding.trim() != "1" {
        let sysctl_path = Path::new("/etc/sysctl.d/90-smp.conf");
        crate::util::atomic_write(sysctl_path, b"net.ipv4.ip_forward = 1\n", 0o644)?;
        let output = Command::new("sysctl")
            .args(["--system"])
            .output()
            .map_err(|error| SmpError::io("sysctl", error))?;
        if !output.status.success() {
            return Err(SmpError::External {
                program: "sysctl --system".to_owned(),
                code: output.status.code().unwrap_or(128),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        changes.push("enabled IPv4 forwarding through /etc/sysctl.d/90-smp.conf".to_owned());
    }
    Ok(changes)
}

pub fn kvm_api_version(path: &Path) -> Result<i32> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| SmpError::io(path.display().to_string(), error))?;
    // SAFETY: KVM_GET_API_VERSION takes no pointer argument and file is a live /dev/kvm fd.
    let result = unsafe { libc::ioctl(file.as_raw_fd(), KVM_GET_API_VERSION) };
    if result < 0 {
        Err(SmpError::io(
            path.display().to_string(),
            std::io::Error::last_os_error(),
        ))
    } else {
        Ok(result)
    }
}

fn command_exists(command: &str) -> bool {
    if command.contains('/') {
        return Path::new(command).is_file();
    }
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths)
            .map(|directory| directory.join(command))
            .any(|candidate| candidate.is_file())
    })
}

fn command_text(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| SmpError::io(program, error))?;
    if !output.status.success() {
        return Err(SmpError::External {
            program: program.to_owned(),
            code: output.status.code().unwrap_or(128),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn path_check(path: &str, name: &str) -> Check {
    match fs::metadata(path) {
        Ok(metadata) => check(
            name,
            true,
            format!("{path} mode {:o}", metadata.mode() & 0o7777),
        ),
        Err(error) => check(name, false, format!("{path}: {error}")),
    }
}

fn check(name: &str, ok: bool, detail: String) -> Check {
    Check {
        name: name.to_owned(),
        ok,
        detail,
    }
}

use std::os::unix::fs::MetadataExt;
