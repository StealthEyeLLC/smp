use crate::error::{Result, SmpError};
use crate::model::RemoteRequest;
use crate::paths::Paths;
use crate::remote::Engine;
use crate::util::ensure_beneath;
use crate::{REQUEST_SCHEMA_VERSION, VERSION};
use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

const MAX_HTTP_BYTES: usize = 2 * 1024 * 1024;
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug)]
pub struct ServeOptions {
    pub socket: PathBuf,
    pub listen: Option<String>,
}

pub fn serve(paths: Paths, options: ServeOptions) -> Result<()> {
    paths.ensure_layout()?;
    ensure_beneath(&paths.runtime, &options.socket)?;
    if options.socket == paths.runtime {
        return Err(SmpError::Invalid(
            "MCP socket must be a file beneath the runtime directory".to_owned(),
        ));
    }
    let engine = Arc::new(Engine::new(paths.clone()));
    engine.reconcile_all()?;
    remove_stale_socket(&options.socket)?;
    let unix = UnixListener::bind(&options.socket)
        .map_err(|error| SmpError::io(options.socket.display().to_string(), error))?;
    fs::set_permissions(&options.socket, fs::Permissions::from_mode(0o660))
        .map_err(|error| SmpError::io(options.socket.display().to_string(), error))?;
    unix.set_nonblocking(true)
        .map_err(|error| SmpError::io(options.socket.display().to_string(), error))?;
    let tcp = options.listen.as_deref().map(bind_loopback).transpose()?;
    install_signal_handlers();
    SHUTDOWN.store(false, Ordering::SeqCst);

    while !SHUTDOWN.load(Ordering::SeqCst) {
        match unix.accept() {
            Ok((stream, _)) => spawn_unix(stream, Arc::clone(&engine)),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => {
                return Err(SmpError::io(options.socket.display().to_string(), error));
            }
        }
        if let Some(listener) = tcp.as_ref() {
            match listener.accept() {
                Ok((stream, _)) => spawn_tcp(stream, Arc::clone(&engine)),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(SmpError::io("SMP loopback listener", error)),
            }
        }
        thread::sleep(Duration::from_millis(25));
    }
    remove_stale_socket(&options.socket)
}

pub fn tool_definition() -> Value {
    json!({
        "name": "go",
        "description": "Execute one versioned SMP operation. Call describe for the live operation catalog.",
        "inputSchema": {
            "type": "object",
            "additionalProperties": false,
            "required": ["schemaVersion", "requestId", "operation"],
            "properties": {
                "schemaVersion": {"type": "integer", "const": REQUEST_SCHEMA_VERSION},
                "requestId": {"type": "string", "minLength": 1, "maxLength": 128},
                "operation": {"type": "string", "minLength": 1, "maxLength": 128},
                "machine": {"type": ["string", "null"]},
                "argv": {"type": ["array", "null"], "items": {"type": "string"}},
                "stdin": {"type": ["string", "null"], "description": "base64 bytes"},
                "timeoutSeconds": {"type": ["integer", "null"], "minimum": 1},
                "outputLimitBytes": {"type": ["integer", "null"], "minimum": 1},
                "detach": {"type": "boolean", "default": false},
                "options": {"type": "object", "additionalProperties": true}
            }
        },
        "outputSchema": {
            "type": "object",
            "required": ["schemaVersion", "requestId", "state"],
            "additionalProperties": true
        }
    })
}

fn spawn_unix(stream: UnixStream, engine: Arc<Engine>) {
    thread::spawn(move || {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
        let _ = handle_http(stream, &engine);
    });
}

fn spawn_tcp(stream: TcpStream, engine: Arc<Engine>) {
    thread::spawn(move || {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
        let _ = handle_http(stream, &engine);
    });
}

fn handle_http<S: Read + Write>(mut stream: S, engine: &Engine) -> Result<()> {
    let request = read_http(&mut stream)?;
    let response = match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/healthz") => http_json(200, json!({"status": "ok", "version": VERSION})),
        ("GET", "/readyz") => match ready(engine) {
            Ok(()) => http_json(200, json!({"status": "ready"})),
            Err(error) => http_json(
                503,
                json!({"status": "not-ready", "error": error.to_string()}),
            ),
        },
        ("POST", "/mcp") | ("POST", "/") => {
            let value: Value = serde_json::from_slice(&request.body)
                .map_err(|error| SmpError::json("<mcp-request>", error))?;
            http_json(200, handle_json_rpc(engine, value))
        }
        _ => http_json(404, json!({"error": "not found"})),
    };
    stream
        .write_all(&response)
        .map_err(|error| SmpError::io("MCP response", error))
}

fn handle_json_rpc(engine: &Engine, request: Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    let outcome = match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "smp", "version": VERSION}
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": [tool_definition()]})),
        "tools/call" => call_tool(engine, params),
        "notifications/initialized" => Ok(json!({})),
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    match outcome {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
        }
    }
}

fn call_tool(engine: &Engine, params: Value) -> std::result::Result<Value, (i32, String)> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    if name != "go" {
        return Err((-32602, format!("unknown tool {name}")));
    }
    let arguments = params
        .get("arguments")
        .cloned()
        .ok_or_else(|| (-32602, "tool arguments are required".to_owned()))?;
    let request: RemoteRequest = serde_json::from_value(arguments)
        .map_err(|error| (-32602, format!("invalid smp.go request: {error}")))?;
    match engine.handle(request) {
        Ok(response) => {
            let text = serde_json::to_string(&response)
                .map_err(|error| (-32603, format!("response serialization failed: {error}")))?;
            let structured = serde_json::to_value(&response)
                .map_err(|error| (-32603, format!("response serialization failed: {error}")))?;
            Ok(json!({
                "content": [{"type": "text", "text": text}],
                "structuredContent": structured,
                "isError": false
            }))
        }
        Err(error) => Ok(json!({
            "content": [{"type": "text", "text": error.to_string()}],
            "isError": true
        })),
    }
}

fn ready(engine: &Engine) -> Result<()> {
    crate::assets::verify(&engine.paths)?;
    engine.reconcile_all()?;
    Ok(())
}

fn bind_loopback(address: &str) -> Result<TcpListener> {
    let socket = address
        .parse::<std::net::SocketAddr>()
        .map_err(|_| SmpError::Invalid(format!("invalid listen address {address}")))?;
    if !socket.ip().is_loopback() {
        return Err(SmpError::Invalid(
            "SMP refuses a non-loopback default listener".to_owned(),
        ));
    }
    let listener =
        TcpListener::bind(socket).map_err(|error| SmpError::io(address.to_owned(), error))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| SmpError::io(address.to_owned(), error))?;
    Ok(listener)
}

fn remove_stale_socket(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            fs::remove_file(path).map_err(|error| SmpError::io(path.display().to_string(), error))
        }
        Ok(_) => Err(SmpError::Ambiguous(format!(
            "refusing to replace non-socket {}",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(SmpError::io(path.display().to_string(), error)),
    }
}

struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

fn read_http<R: Read>(reader: &mut R) -> Result<HttpRequest> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        if bytes.len() >= MAX_HTTP_BYTES {
            return Err(SmpError::Invalid("HTTP request exceeds limit".to_owned()));
        }
        let count = reader
            .read(&mut buffer)
            .map_err(|error| SmpError::io("MCP request", error))?;
        if count == 0 {
            return Err(SmpError::Invalid("incomplete HTTP request".to_owned()));
        }
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let head = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| SmpError::Invalid("HTTP headers are not UTF-8".to_owned()))?;
    let mut lines = head.split("\r\n");
    let mut start = lines
        .next()
        .ok_or_else(|| SmpError::Invalid("HTTP start line is missing".to_owned()))?
        .split_whitespace();
    let method = start.next().unwrap_or("").to_owned();
    let path = start.next().unwrap_or("").to_owned();
    if start.next().is_none() || !matches!(method.as_str(), "GET" | "POST") {
        return Err(SmpError::Invalid("invalid HTTP start line".to_owned()));
    }
    let content_length = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim().parse::<usize>())
        .transpose()
        .map_err(|_| SmpError::Invalid("invalid Content-Length".to_owned()))?
        .unwrap_or(0);
    if header_end.saturating_add(content_length) > MAX_HTTP_BYTES {
        return Err(SmpError::Invalid("HTTP body exceeds limit".to_owned()));
    }
    while bytes.len() < header_end + content_length {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| SmpError::io("MCP request", error))?;
        if count == 0 {
            return Err(SmpError::Invalid("incomplete HTTP body".to_owned()));
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(HttpRequest {
        method,
        path,
        body: bytes[header_end..header_end + content_length].to_vec(),
    })
}

fn http_json(status: u16, value: Value) -> Vec<u8> {
    let body =
        serde_json::to_vec(&value).unwrap_or_else(|_| b"{\"error\":\"serialization\"}".to_vec());
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(&body);
    response
}

extern "C" fn signal_shutdown(_: i32) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

fn install_signal_handlers() {
    // SAFETY: the handler only stores to a lock-free atomic and has C signal-handler ABI.
    unsafe {
        libc::signal(libc::SIGTERM, signal_shutdown as libc::sighandler_t);
        libc::signal(libc::SIGINT, signal_shutdown as libc::sighandler_t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_one_tool_is_exposed() {
        let tools = json!([tool_definition()]);
        assert_eq!(tools.as_array().map(Vec::len), Some(1));
        assert_eq!(tools[0]["name"], "go");
        assert!(
            tools[0]["inputSchema"]["properties"]["operation"]
                .get("enum")
                .is_none()
        );
    }

    #[test]
    fn non_loopback_binding_is_rejected() {
        assert!(bind_loopback("0.0.0.0:0").is_err());
    }

    #[test]
    fn json_rpc_lists_only_smp_go() -> Result<()> {
        let directory = tempfile::tempdir().map_err(|error| SmpError::io("tempdir", error))?;
        let paths = Paths::rooted(directory.path())?;
        let value = handle_json_rpc(
            &Engine::new(paths),
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        );
        assert_eq!(value["result"]["tools"][0]["name"], "go");
        assert_eq!(value["result"]["tools"].as_array().map(Vec::len), Some(1));
        Ok(())
    }
}
