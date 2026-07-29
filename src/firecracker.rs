use crate::error::{Result, SmpError};
use crate::model::{MachineRecord, ProcessIdentity, Transport};
use crate::process;
use crate::util::{atomic_json, sha256_file};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
struct FirecrackerConfiguration {
    boot_source: BootSource,
    drives: Vec<Drive>,
    network_interfaces: Vec<NetworkInterface>,
    machine_config: MachineConfiguration,
    entropy: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct BootSource {
    kernel_image_path: PathBuf,
    boot_args: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    initrd_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct Drive {
    drive_id: String,
    path_on_host: PathBuf,
    is_root_device: bool,
    is_read_only: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct NetworkInterface {
    iface_id: String,
    host_dev_name: String,
    guest_mac: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct MachineConfiguration {
    vcpu_count: u8,
    mem_size_mib: u32,
    smt: bool,
    track_dirty_pages: bool,
}

#[derive(Clone, Debug)]
pub struct ApiResponse {
    pub status_code: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

pub fn write_configuration(record: &MachineRecord) -> Result<(PathBuf, String)> {
    record.validate()?;
    let mut drives = vec![
        Drive {
            drive_id: record.root_disk.id.clone(),
            path_on_host: record.root_disk.path.clone(),
            is_root_device: true,
            is_read_only: record.root_disk.read_only,
        },
        Drive {
            drive_id: "seed".to_owned(),
            path_on_host: record.seed_path.clone(),
            is_root_device: false,
            is_read_only: true,
        },
    ];
    drives.extend(record.additional_disks.iter().map(|disk| Drive {
        drive_id: disk.id.clone(),
        path_on_host: disk.path.clone(),
        is_root_device: disk.is_root,
        is_read_only: disk.read_only,
    }));
    let configuration = FirecrackerConfiguration {
        boot_source: BootSource {
            kernel_image_path: record.kernel_path.clone(),
            boot_args: record.kernel_arguments.clone(),
            initrd_path: record.initrd_path.clone(),
        },
        drives,
        network_interfaces: vec![NetworkInterface {
            iface_id: "eth0".to_owned(),
            host_dev_name: record.network.tap.clone(),
            guest_mac: record.network.guest_mac.clone(),
        }],
        machine_config: MachineConfiguration {
            vcpu_count: record.vcpu_count,
            mem_size_mib: record.memory_mib,
            smt: false,
            track_dirty_pages: false,
        },
        entropy: BTreeMap::new(),
    };
    let path = record.machine_directory.join("firecracker.json");
    atomic_json(&path, &configuration, 0o600)?;
    Ok((path.clone(), sha256_file(&path)?))
}

pub fn launch(record: &MachineRecord) -> Result<ProcessIdentity> {
    if record.api_socket.exists() {
        return Err(SmpError::Ambiguous(format!(
            "API socket already exists: {}",
            record.api_socket.display()
        )));
    }
    let (config, _) = write_configuration(record)?;
    let mut args = vec![
        "--api-sock".to_owned(),
        record.api_socket.display().to_string(),
        "--config-file".to_owned(),
        config.display().to_string(),
    ];
    if record.transport == Transport::Pci {
        args.push("--enable-pci".to_owned());
    }
    let stdout = record.machine_directory.join("firecracker.stdout.log");
    let stderr = record.machine_directory.join("firecracker.stderr.log");
    let (_child, identity) = process::spawn_detached(
        &record.firecracker_path,
        &args,
        &record.machine_directory,
        &stdout,
        &stderr,
    )?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if !process::is_running(&identity) {
            return Err(SmpError::State(format!(
                "Firecracker exited before API socket creation; see {}",
                stderr.display()
            )));
        }
        if record.api_socket.exists() {
            process::verified_socket(&identity, &record.machine_directory, &record.api_socket)?;
            return Ok(identity);
        }
        if Instant::now() >= deadline {
            let _ = process::signal(&identity, libc::SIGTERM);
            return Err(SmpError::State(
                "Firecracker API socket did not become ready".to_owned(),
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub fn raw_api(
    record: &MachineRecord,
    method: &str,
    path: &str,
    headers: &BTreeMap<String, String>,
    body: &[u8],
) -> Result<ApiResponse> {
    let identity = record
        .firecracker_process
        .as_ref()
        .ok_or_else(|| SmpError::State("machine has no Firecracker process".to_owned()))?;
    process::verified_socket(identity, &record.machine_directory, &record.api_socket)?;
    if !matches!(method, "GET" | "PUT" | "PATCH" | "DELETE")
        || !path.starts_with('/')
        || path.contains('\r')
        || path.contains('\n')
    {
        return Err(SmpError::Invalid(
            "invalid Firecracker API method or path".to_owned(),
        ));
    }
    let mut stream = UnixStream::connect(&record.api_socket)
        .map_err(|error| SmpError::io(record.api_socket.display().to_string(), error))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| SmpError::io(record.api_socket.display().to_string(), error))?;
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: localhost\r\n")
        .map_err(|error| SmpError::io(record.api_socket.display().to_string(), error))?;
    for (name, value) in headers {
        if name.contains('\r')
            || name.contains('\n')
            || name.contains(':')
            || value.contains('\r')
            || value.contains('\n')
        {
            return Err(SmpError::Invalid(
                "invalid Firecracker API header".to_owned(),
            ));
        }
        write!(stream, "{name}: {value}\r\n")
            .map_err(|error| SmpError::io(record.api_socket.display().to_string(), error))?;
    }
    write!(stream, "Content-Length: {}\r\n\r\n", body.len())
        .map_err(|error| SmpError::io(record.api_socket.display().to_string(), error))?;
    stream
        .write_all(body)
        .map_err(|error| SmpError::io(record.api_socket.display().to_string(), error))?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|error| SmpError::io(record.api_socket.display().to_string(), error))?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|error| SmpError::io(record.api_socket.display().to_string(), error))?;
    parse_http_response(&response)
}

pub fn remove_stale_socket(record: &MachineRecord) -> Result<()> {
    if let Some(identity) = &record.firecracker_process
        && process::is_running(identity)
    {
        return Err(SmpError::Ambiguous(
            "refusing to remove an API socket for a live verified process".to_owned(),
        ));
    }
    match fs::remove_file(&record.api_socket) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(SmpError::io(record.api_socket.display().to_string(), error)),
    }
}

fn parse_http_response(response: &[u8]) -> Result<ApiResponse> {
    let boundary = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| SmpError::State("malformed Firecracker API response".to_owned()))?;
    let head = std::str::from_utf8(&response[..boundary])
        .map_err(|_| SmpError::State("non-UTF-8 Firecracker response headers".to_owned()))?;
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| SmpError::State("missing Firecracker response status".to_owned()))?;
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| SmpError::State("invalid Firecracker response status".to_owned()))?;
    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    Ok(ApiResponse {
        status_code,
        headers,
        body: response[boundary + 4..].to_vec(),
    })
}

pub fn configuration_digest(record: &MachineRecord) -> Result<String> {
    sha256_file(&record.machine_directory.join("firecracker.json"))
}

pub fn expected_binary_digest(path: &Path) -> Result<String> {
    sha256_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_firecracker_http_response() -> Result<()> {
        let response = parse_http_response(
            b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nX-Test: yes\r\n\r\n",
        )?;
        assert_eq!(response.status_code, 204);
        assert_eq!(
            response.headers.get("x-test").map(String::as_str),
            Some("yes")
        );
        assert!(response.body.is_empty());
        Ok(())
    }

    #[test]
    fn malformed_response_is_rejected() {
        assert!(parse_http_response(b"not-http").is_err());
    }
}
