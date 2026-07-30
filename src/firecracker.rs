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

const API_IO_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_API_HEADER_BYTES: usize = 64 * 1024;
const MAX_API_RESPONSE_BYTES: usize = 1024 * 1024;

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

#[derive(Debug)]
struct ParsedHead {
    status_code: u16,
    headers: BTreeMap<String, String>,
    content_length: Option<usize>,
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

pub fn launch(record: &MachineRecord, runtime: &Path) -> Result<ProcessIdentity> {
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
            process::verified_socket(&identity, runtime, &record.api_socket)?;
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
    runtime: &Path,
    method: &str,
    path: &str,
    headers: &BTreeMap<String, String>,
    body: &[u8],
) -> Result<ApiResponse> {
    let identity = record
        .firecracker_process
        .as_ref()
        .ok_or_else(|| SmpError::State("machine has no Firecracker process".to_owned()))?;
    process::verified_socket(identity, runtime, &record.api_socket)?;
    validate_api_request(method, path, headers)?;

    let context = record.api_socket.display().to_string();
    let mut stream = UnixStream::connect(&record.api_socket)
        .map_err(|error| SmpError::io(context.clone(), error))?;
    stream
        .set_read_timeout(Some(API_IO_TIMEOUT))
        .map_err(|error| SmpError::io(context.clone(), error))?;
    stream
        .set_write_timeout(Some(API_IO_TIMEOUT))
        .map_err(|error| SmpError::io(context.clone(), error))?;
    send_http_request(&mut stream, &context, method, path, headers, body)?;
    read_http_response(&mut stream, &context)
}

fn validate_api_request(
    method: &str,
    path: &str,
    headers: &BTreeMap<String, String>,
) -> Result<()> {
    if !matches!(method, "GET" | "PUT" | "PATCH" | "DELETE") || !valid_api_path(path) {
        return Err(SmpError::Invalid(
            "invalid Firecracker API method or path".to_owned(),
        ));
    }
    for (name, value) in headers {
        if !valid_header_name(name) || !valid_header_value(value) {
            return Err(SmpError::Invalid(
                "invalid Firecracker API header".to_owned(),
            ));
        }
        if matches!(
            name.to_ascii_lowercase().as_str(),
            "host" | "content-length" | "connection"
        ) {
            return Err(SmpError::Invalid(format!(
                "caller cannot override Firecracker API framing header {name}"
            )));
        }
    }
    Ok(())
}

fn valid_api_path(path: &str) -> bool {
    if !path.starts_with('/')
        || !path.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        || path
            .bytes()
            .any(|byte| matches!(byte, b'%' | b'\\' | b'?' | b'#'))
    {
        return false;
    }
    if path == "/" {
        return true;
    }
    path[1..]
        .split('/')
        .all(|segment| !segment.is_empty() && !matches!(segment, "." | ".."))
}

fn valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn valid_header_value(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte == b'\t' || (byte >= 0x20 && byte != 0x7f))
}

fn send_http_request<W: Write>(
    writer: &mut W,
    context: &str,
    method: &str,
    path: &str,
    headers: &BTreeMap<String, String>,
    body: &[u8],
) -> Result<()> {
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    )
    .into_bytes();
    for (name, value) in headers {
        request.extend_from_slice(name.as_bytes());
        request.extend_from_slice(b": ");
        request.extend_from_slice(value.as_bytes());
        request.extend_from_slice(b"\r\n");
    }
    request.extend_from_slice(b"\r\n");
    request.extend_from_slice(body);
    writer
        .write_all(&request)
        .map_err(|error| SmpError::io(context.to_owned(), error))?;
    writer
        .flush()
        .map_err(|error| SmpError::io(context.to_owned(), error))
}

fn read_http_response<R: Read>(reader: &mut R, context: &str) -> Result<ApiResponse> {
    let mut response = Vec::with_capacity(4096);
    let boundary = loop {
        if let Some(boundary) = header_boundary(&response) {
            break boundary;
        }
        if response.len() >= MAX_API_HEADER_BYTES {
            return Err(SmpError::State(format!(
                "Firecracker API response headers exceed {MAX_API_HEADER_BYTES} bytes"
            )));
        }
        let count = read_bounded(
            reader,
            context,
            &mut response,
            MAX_API_HEADER_BYTES.min(MAX_API_RESPONSE_BYTES),
        )?;
        if count == 0 {
            return Err(SmpError::State(
                "incomplete Firecracker API response headers".to_owned(),
            ));
        }
    };
    if boundary > MAX_API_HEADER_BYTES {
        return Err(SmpError::State(format!(
            "Firecracker API response headers exceed {MAX_API_HEADER_BYTES} bytes"
        )));
    }

    let body_offset = boundary + 4;
    let parsed = parse_http_head(&response[..boundary])?;
    if response_has_no_body(parsed.status_code) {
        if parsed.content_length.is_some_and(|length| length != 0) {
            return Err(SmpError::State(
                "bodyless Firecracker API response declared a nonzero body".to_owned(),
            ));
        }
        return Ok(ApiResponse {
            status_code: parsed.status_code,
            headers: parsed.headers,
            body: Vec::new(),
        });
    }

    let body = if let Some(length) = parsed.content_length {
        let total = body_offset.checked_add(length).ok_or_else(|| {
            SmpError::State("Firecracker API response length overflow".to_owned())
        })?;
        if total > MAX_API_RESPONSE_BYTES {
            return Err(SmpError::State(format!(
                "Firecracker API response exceeds {MAX_API_RESPONSE_BYTES} bytes"
            )));
        }
        while response.len() < total {
            let count = read_bounded(reader, context, &mut response, total)?;
            if count == 0 {
                return Err(SmpError::State(
                    "truncated Firecracker API response body".to_owned(),
                ));
            }
        }
        response[body_offset..total].to_vec()
    } else {
        while response.len() < MAX_API_RESPONSE_BYTES {
            let count = read_bounded(reader, context, &mut response, MAX_API_RESPONSE_BYTES)?;
            if count == 0 {
                break;
            }
        }
        if response.len() == MAX_API_RESPONSE_BYTES {
            let mut extra = [0_u8; 1];
            let count = reader
                .read(&mut extra)
                .map_err(|error| SmpError::io(context.to_owned(), error))?;
            if count != 0 {
                return Err(SmpError::State(format!(
                    "Firecracker API response exceeds {MAX_API_RESPONSE_BYTES} bytes"
                )));
            }
        }
        response[body_offset..].to_vec()
    };

    Ok(ApiResponse {
        status_code: parsed.status_code,
        headers: parsed.headers,
        body,
    })
}

fn read_bounded<R: Read>(
    reader: &mut R,
    context: &str,
    destination: &mut Vec<u8>,
    limit: usize,
) -> Result<usize> {
    if destination.len() >= limit {
        return Ok(0);
    }
    let mut chunk = [0_u8; 8192];
    let available = (limit - destination.len()).min(chunk.len());
    let count = reader
        .read(&mut chunk[..available])
        .map_err(|error| SmpError::io(context.to_owned(), error))?;
    destination.extend_from_slice(&chunk[..count]);
    Ok(count)
}

fn header_boundary(response: &[u8]) -> Option<usize> {
    response.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_http_head(head: &[u8]) -> Result<ParsedHead> {
    let head = std::str::from_utf8(head)
        .map_err(|_| SmpError::State("non-UTF-8 Firecracker response headers".to_owned()))?;
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| SmpError::State("missing Firecracker response status".to_owned()))?;
    let mut status_parts = status_line.split_whitespace();
    let version = status_parts
        .next()
        .ok_or_else(|| SmpError::State("missing Firecracker HTTP version".to_owned()))?;
    let code = status_parts
        .next()
        .ok_or_else(|| SmpError::State("missing Firecracker response status".to_owned()))?;
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1")
        || code.len() != 3
        || !code.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(SmpError::State(
            "invalid Firecracker response status".to_owned(),
        ));
    }
    let status_code = code
        .parse::<u16>()
        .ok()
        .filter(|value| (100..=599).contains(value))
        .ok_or_else(|| SmpError::State("invalid Firecracker response status".to_owned()))?;

    let mut headers = BTreeMap::<String, String>::new();
    let mut content_length = None;
    let mut transfer_encoding = None;
    for line in lines {
        if line.is_empty() || line.starts_with(' ') || line.starts_with('\t') {
            return Err(SmpError::State(
                "malformed Firecracker response header".to_owned(),
            ));
        }
        let (name, raw_value) = line
            .split_once(':')
            .ok_or_else(|| SmpError::State("malformed Firecracker response header".to_owned()))?;
        let value = raw_value.trim();
        if !valid_header_name(name) || !valid_header_value(value) {
            return Err(SmpError::State(
                "invalid Firecracker response header".to_owned(),
            ));
        }
        let normalized = name.to_ascii_lowercase();
        match normalized.as_str() {
            "content-length" => {
                if content_length.is_some() || value.contains(',') {
                    return Err(SmpError::State(
                        "ambiguous Firecracker response content length".to_owned(),
                    ));
                }
                content_length = Some(value.parse::<usize>().map_err(|_| {
                    SmpError::State("invalid Firecracker response content length".to_owned())
                })?);
            }
            "transfer-encoding" => {
                if transfer_encoding.is_some() {
                    return Err(SmpError::State(
                        "ambiguous Firecracker response transfer encoding".to_owned(),
                    ));
                }
                transfer_encoding = Some(value.to_ascii_lowercase());
            }
            _ => {}
        }
        headers
            .entry(normalized)
            .and_modify(|existing| {
                existing.push_str(", ");
                existing.push_str(value);
            })
            .or_insert_with(|| value.to_owned());
    }
    if let Some(encoding) = transfer_encoding {
        if encoding != "identity" || content_length.is_some() {
            return Err(SmpError::State(format!(
                "unsupported Firecracker response transfer encoding {encoding}"
            )));
        }
    }
    Ok(ParsedHead {
        status_code,
        headers,
        content_length,
    })
}

fn response_has_no_body(status_code: u16) -> bool {
    (100..200).contains(&status_code) || matches!(status_code, 204 | 304)
}

#[cfg(test)]
fn parse_http_response(response: &[u8]) -> Result<ApiResponse> {
    read_http_response(
        &mut std::io::Cursor::new(response),
        "<Firecracker response>",
    )
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

pub fn configuration_digest(record: &MachineRecord) -> Result<String> {
    sha256_file(&record.machine_directory.join("firecracker.json"))
}

pub fn expected_binary_digest(path: &Path) -> Result<String> {
    sha256_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;

    fn exchange_with_server(response: Vec<u8>, body: Vec<u8>) -> ApiResponse {
        let directory = tempfile::tempdir().expect("temporary directory");
        let socket = directory.path().join("firecracker.sock");
        let listener = UnixListener::bind(&socket).expect("bind test socket");
        let (ready_tx, ready_rx) = mpsc::channel();
        let expected_body = body.clone();
        let server = thread::spawn(move || {
            ready_tx.send(()).expect("signal listener readiness");
            let (mut stream, _) = listener.accept().expect("accept test client");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("set server read timeout");
            let mut request = Vec::new();
            let boundary = loop {
                if let Some(boundary) = header_boundary(&request) {
                    break boundary;
                }
                let mut chunk = [0_u8; 1024];
                let count = stream.read(&mut chunk).expect("read request headers");
                assert_ne!(count, 0, "client half-closed before request headers");
                request.extend_from_slice(&chunk[..count]);
            };
            let head = std::str::from_utf8(&request[..boundary]).expect("UTF-8 request headers");
            let mut lines = head.split("\r\n");
            assert_eq!(lines.next(), Some("PUT /machine-config HTTP/1.1"));
            let headers = lines
                .map(|line| line.split_once(':').expect("request header"))
                .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
                .collect::<BTreeMap<_, _>>();
            assert_eq!(headers.get("host").map(String::as_str), Some("localhost"));
            assert_eq!(headers.get("connection").map(String::as_str), Some("close"));
            let expected_length = expected_body.len().to_string();
            assert_eq!(
                headers.get("content-length").map(String::as_str),
                Some(expected_length.as_str())
            );
            let expected_total = boundary + 4 + expected_body.len();
            while request.len() < expected_total {
                let mut chunk = [0_u8; 1024];
                let count = stream.read(&mut chunk).expect("read request body");
                assert_ne!(count, 0, "client half-closed before request body");
                request.extend_from_slice(&chunk[..count]);
            }
            assert_eq!(
                &request[boundary + 4..expected_total],
                expected_body.as_slice()
            );

            stream
                .set_read_timeout(Some(Duration::from_millis(100)))
                .expect("set half-close probe timeout");
            let mut probe = [0_u8; 1];
            match stream.read(&mut probe) {
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Ok(0) => panic!("client prematurely half-closed its write side"),
                Ok(_) => panic!("client sent bytes beyond Content-Length"),
                Err(error) => panic!("unexpected half-close probe error: {error}"),
            }
            stream.write_all(&response).expect("write test response");
            stream.flush().expect("flush test response");
            thread::sleep(Duration::from_millis(200));
        });
        ready_rx.recv().expect("listener ready");

        let mut stream = UnixStream::connect(&socket).expect("connect test socket");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set client read timeout");
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .expect("set client write timeout");
        let headers = BTreeMap::from([("Content-Type".to_owned(), "application/json".to_owned())]);
        validate_api_request("PUT", "/machine-config", &headers).expect("valid request");
        send_http_request(
            &mut stream,
            "<test socket>",
            "PUT",
            "/machine-config",
            &headers,
            &body,
        )
        .expect("send request");
        let parsed = read_http_response(&mut stream, "<test socket>").expect("parse response");
        server.join().expect("test server");
        parsed
    }

    #[test]
    fn unix_client_sends_complete_request_without_half_close_and_parses_json() {
        let response_body = br#"{"vcpu_count":2,"mem_size_mib":1024}"#;
        let response = exchange_with_server(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Test: yes\r\n\r\n{}",
                response_body.len(),
                std::str::from_utf8(response_body).expect("JSON response")
            )
            .into_bytes(),
            br#"{"vcpu_count":2}"#.to_vec(),
        );
        assert_eq!(response.status_code, 200);
        assert_eq!(
            response.headers.get("x-test").map(String::as_str),
            Some("yes")
        );
        let value: Value = serde_json::from_slice(&response.body).expect("JSON body");
        assert_eq!(value["vcpu_count"], 2);
        assert_eq!(value["mem_size_mib"], 1024);
    }

    #[test]
    fn unix_client_parses_204_without_waiting_for_eof() {
        let response = exchange_with_server(
            b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n".to_vec(),
            Vec::new(),
        );
        assert_eq!(response.status_code, 204);
        assert!(response.body.is_empty());
    }

    #[test]
    fn non_success_body_is_preserved() -> Result<()> {
        let response = parse_http_response(
            b"HTTP/1.1 400 Bad Request\r\nContent-Length: 15\r\n\r\n{\"fault\":\"bad\"}",
        )?;
        assert_eq!(response.status_code, 400);
        assert_eq!(response.body, br#"{"fault":"bad"}"#);
        Ok(())
    }

    #[test]
    fn oversized_response_is_rejected() {
        let response =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {MAX_API_RESPONSE_BYTES}\r\n\r\n");
        assert!(parse_http_response(response.as_bytes()).is_err());
    }

    #[test]
    fn malformed_response_is_rejected() {
        assert!(parse_http_response(b"not-http").is_err());
        assert!(parse_http_response(b"HTTP/1.1 200 OK\r\nBroken\r\n\r\n").is_err());
        assert!(parse_http_response(b"HTTP/1.1 200 OK\r\nContent-Length: nope\r\n\r\n").is_err());
    }

    #[test]
    fn canonical_firecracker_api_paths_are_accepted() {
        for path in [
            "/",
            "/machine-config",
            "/boot-source",
            "/drives/root",
            "/network-interfaces/eth0",
            "/actions",
            "/metrics",
            "/snapshot/create",
            "/snapshot/load",
        ] {
            assert!(valid_api_path(path), "expected canonical path: {path:?}");
            validate_api_request("GET", path, &BTreeMap::new())
                .expect("canonical Firecracker API path");
        }
    }

    #[test]
    fn noncanonical_firecracker_api_paths_are_rejected() {
        for path in [
            "machine-config",
            "../machine-config",
            "/../machine-config",
            "/./machine-config",
            "/foo/../machine-config",
            "/foo/./machine-config",
            "/..",
            "/.",
            "/a\\..\\b",
            "/..\\machine-config",
            "/%2e%2e/machine-config",
            "/%2E%2E/machine-config",
            "/%2e./machine-config",
            "/.%2e/machine-config",
            "/%5c../machine-config",
            "/%255c../machine-config",
            "/%252e%252e/machine-config",
            "/machine-config?x=1",
            "/machine-config#fragment",
            "/machine-config\r",
            "/machine-config\n",
            "/machine-config\0",
            "/machine-config\u{1f}",
            "//machine-config",
            "/machine-config/",
        ] {
            assert!(!valid_api_path(path), "expected rejection: {path:?}");
            let error = validate_api_request("GET", path, &BTreeMap::new())
                .expect_err("noncanonical Firecracker API path must fail locally");
            assert!(
                matches!(error, SmpError::Invalid(_)),
                "path={path:?} error={error:?}"
            );
        }
    }

    #[test]
    fn request_injection_and_framing_overrides_are_rejected() {
        assert!(
            validate_api_request("GET", "/machine-config\r\nInjected: yes", &BTreeMap::new())
                .is_err()
        );
        assert!(
            validate_api_request(
                "GET",
                "/machine-config",
                &BTreeMap::from([("Content-Length".to_owned(), "7".to_owned())])
            )
            .is_err()
        );
    }
}
