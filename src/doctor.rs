use crate::assets;
use crate::state::RuntimePaths;
use crate::util::{command_exists, os_strings, run};
use anyhow::{Context, Result};
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::Command;

const KVM_GET_API_VERSION: libc::c_ulong = 0xAE00;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorCheck {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    pub fixable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    pub healthy: bool,
    pub changed: Vec<String>,
    pub checks: Vec<DoctorCheck>,
}

pub fn run_doctor(paths: &RuntimePaths, fix: bool) -> Result<DoctorReport> {
    let mut changed = Vec::new();
    if fix {
        paths.ensure()?;
        changed.push(format!("ensured SMP runtime directories beneath {}", paths.state_root.display()));
        if !Path::new("/dev/net/tun").exists() && command_exists("modprobe") {
            if run("modprobe", &os_strings(&["tun"])).is_ok() {
                changed.push("loaded the tun kernel module".to_owned());
            }
        }
        let forwarding = fs::read_to_string("/proc/sys/net/ipv4/ip_forward").unwrap_or_default();
        if forwarding.trim() != "1" {
            fs::write("/proc/sys/net/ipv4/ip_forward", b"1\n")
                .context("enable IPv4 forwarding")?;
            changed.push("enabled net.ipv4.ip_forward".to_owned());
        }
    }

    let mut checks = Vec::new();
    let architecture = std::env::consts::ARCH.to_owned();
    checks.push(DoctorCheck {
        name: "architecture".to_owned(),
        ok: architecture == "x86_64",
        detail: architecture,
        fixable: false,
    });

    checks.push(kvm_check());
    checks.push(path_check("tunTap", "/dev/net/tun", true));
    checks.push(command_check("nftables", "nft"));
    checks.push(command_check("iproute2", "ip"));
    checks.push(command_check("ssh", "ssh"));
    checks.push(command_check("scp", "scp"));
    checks.push(command_check("curl", "curl"));
    checks.push(command_check("mkfsExt4", "mkfs.ext4"));
    checks.push(command_check("mount", "mount"));
    checks.push(command_check("losetup", "losetup"));

    let forwarding = fs::read_to_string("/proc/sys/net/ipv4/ip_forward").unwrap_or_default();
    checks.push(DoctorCheck {
        name: "ipForwarding".to_owned(),
        ok: forwarding.trim() == "1",
        detail: forwarding.trim().to_owned(),
        fixable: true,
    });

    let memory_kib = fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("MemAvailable:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|value| value.parse::<u64>().ok())
        })
        .unwrap_or(0);
    checks.push(DoctorCheck {
        name: "memory".to_owned(),
        ok: memory_kib >= 1_048_576,
        detail: format!("{memory_kib} KiB available"),
        fixable: false,
    });

    let disk = Command::new("df")
        .args(["-Pk", paths.state_root.to_string_lossy().as_ref()])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .unwrap_or_else(|| "unavailable".to_owned());
    checks.push(DoctorCheck {
        name: "disk".to_owned(),
        ok: disk != "unavailable",
        detail: disk.lines().last().unwrap_or("unavailable").to_owned(),
        fixable: false,
    });

    let manifest = assets::load_manifest(paths);
    checks.push(DoctorCheck {
        name: "assets".to_owned(),
        ok: manifest.is_ok(),
        detail: manifest
            .map(|value| format!(
                "Firecracker {}, Linux {}, Debian {} {}",
                value.firecracker.version, value.kernel.version, value.debian_version, value.debian_suite
            ))
            .unwrap_or_else(|error| error.to_string()),
        fixable: true,
    });

    checks.push(service_check("smpService", "smp.service"));
    checks.push(service_check("smpTunnelService", "smp-tunnel.service"));

    let healthy = checks.iter().all(|check| check.ok || check.name == "smpTunnelService");
    Ok(DoctorReport {
        healthy,
        changed,
        checks,
    })
}

fn kvm_check() -> DoctorCheck {
    let result = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .and_then(|file| {
            let version = unsafe { libc::ioctl(file.as_raw_fd(), KVM_GET_API_VERSION) };
            if version == 12 {
                Ok(version)
            } else if version < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    format!("unexpected KVM API version {version}"),
                ))
            }
        });
    DoctorCheck {
        name: "kvm".to_owned(),
        ok: result.is_ok(),
        detail: result
            .map(|version| format!("KVM API version {version}"))
            .unwrap_or_else(|error| error.to_string()),
        fixable: false,
    }
}

fn path_check(name: &str, path: &str, fixable: bool) -> DoctorCheck {
    DoctorCheck {
        name: name.to_owned(),
        ok: Path::new(path).exists(),
        detail: path.to_owned(),
        fixable,
    }
}

fn command_check(name: &str, command: &str) -> DoctorCheck {
    DoctorCheck {
        name: name.to_owned(),
        ok: command_exists(command),
        detail: command.to_owned(),
        fixable: false,
    }
}

fn service_check(name: &str, service: &str) -> DoctorCheck {
    let output = Command::new("systemctl")
        .args(["is-active", service])
        .output();
    let (ok, detail) = match output {
        Ok(output) => (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        ),
        Err(error) => (false, error.to_string()),
    };
    DoctorCheck {
        name: name.to_owned(),
        ok,
        detail,
        fixable: false,
    }
}
