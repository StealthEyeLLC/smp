use crate::model::{GoRequest, REQUEST_SCHEMA_VERSION};
use crate::remote;
use crate::state::RuntimePaths;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
const MAX_HTTP_BODY: usize = 2 * 1024 * 1024;

pub fn serve(paths: RuntimePaths, listen: SocketAddr) -> Result<()> {
    if !listen.ip().is_loopback() {
        bail!(
            "smp serve refuses a non-loopback listener; use the dedicated authenticated SMP tunnel"
        );
    }
    paths.ensure()?;
    let listener =
        TcpListener::bind(listen).with_context(|| format!("bind SMP MCP listener {listen}"))?;
    for connection in listener.incoming() {
        match connection {
            Ok(mut stream) => {
                stream.set_read_timeout(Some(Duration::from_secs(30)))?;
                stream.set_write_timeout(Some(Duration::from_secs(30)))?;
                if let Err(error) = handle_connection(&paths, &mut stream) {
                    let body = json!({"error": error.to_string()}).to_string();
                    let _ = write_response(&mut stream, 500, "application/json", body.as_bytes());
                }
            }
            Err(error) => eprintln!("smp serve accept error: {error}"),
        }
    }
    Ok(())
}

fn handle_connection(paths: &RuntimePaths, stream: &mut TcpStream) -> Result<()> {
    let request = read_http_request(stream)?;
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/healthz") => {
            write_response(stream, 200, "application/json", br#"{"healthy":true}"#)
        }
        ("GET", "/readyz") => {
            let ready = remote::describe(paths, false).is_ok();
            let status = if ready { 200 } else { 503 };
            write_response(
                stream,
                status,
                "application/json",
                json!({"ready": ready}).to_string().as_bytes(),
            )
        }
        ("POST", "/mcp") => handle_mcp(paths, stream, &request.body),
        _ => write_response(stream, 404, "application/json", br#"{"error":"not found"}"#),
    }
}

fn handle_mcp(paths: &RuntimePaths, stream: &mut TcpStream, body: &[u8]) -> Result<()> {
    let value: Value = serde_json::from_slice(body).context("parse JSON-RPC request")?;
    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
    let id = value.get("id").cloned().unwrap_or(Value::Null);
    if id.is_null() && method.starts_with("notifications/") {
        return write_response(stream, 202, "application/json", b"");
    }
    let response = match method {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "smp", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Call describe first. SMP exposes exactly one tool, go."
            }
        }),
        "ping" => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
        "tools/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": [{
                    "name": "go",
                    "title": "SMP Go",
                    "description": "Operate standalone SMP through one broad versioned request envelope. Call operation=describe first.",
                    "inputSchema": go_input_schema()
                }]
            }
        }),
        "tools/call" => {
            let params = value.get("params").and_then(Value::as_object);
            let tool = params
                .and_then(|params| params.get("name"))
                .and_then(Value::as_str);
            if tool != Some("go") {
                json_rpc_error(id, -32602, "SMP exposes exactly one tool named go")
            } else {
                let arguments = params
                    .and_then(|params| params.get("arguments"))
                    .cloned()
                    .unwrap_or(Value::Null);
                match serde_json::from_value::<GoRequest>(arguments) {
                    Ok(request) => {
                        let result = remote::handle_go(paths, request);
                        let is_error = matches!(result.state.as_str(), "failed" | "cancelled");
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "content": [{"type": "text", "text": serde_json::to_string(&result)?}],
                                "structuredContent": result,
                                "isError": is_error
                            }
                        })
                    }
                    Err(error) => {
                        json_rpc_error(id, -32602, &format!("invalid smp.go request: {error}"))
                    }
                }
            }
        }
        _ => json_rpc_error(id, -32601, "method not found"),
    };
    let bytes = serde_json::to_vec(&response)?;
    write_response(stream, 200, "application/json", &bytes)
}

fn go_input_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["schemaVersion", "requestId", "operation"],
        "properties": {
            "schemaVersion": {"type": "integer", "const": REQUEST_SCHEMA_VERSION},
            "requestId": {"type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[A-Za-z0-9_-]+$"},
            "operation": {"type": "string", "minLength": 1, "maxLength": 128},
            "machine": {"type": ["string", "null"]},
            "argv": {"type": "array", "items": {"type": "string"}, "maxItems": 4096, "default": []},
            "stdin": {"type": ["string", "null"], "default": null},
            "timeoutSeconds": {"type": ["integer", "null"], "minimum": 1, "maximum": 86400, "default": 300},
            "outputLimitBytes": {"type": ["integer", "null"], "minimum": 1, "maximum": 67108864, "default": 1048576},
            "detach": {"type": "boolean", "default": false},
            "options": {"type": "object", "additionalProperties": true, "default": {}}
        }
    })
}

fn json_rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    let header_end = loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            bail!("connection closed before HTTP headers completed");
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > 65_536 {
            bail!("HTTP headers exceed 64 KiB");
        }
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head =
        String::from_utf8(bytes[..header_end].to_vec()).context("HTTP headers are not UTF-8")?;
    let mut lines = head.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing request line"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let path = parts
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
        .to_owned();
    let content_length = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim().parse::<usize>())
        .transpose()?
        .unwrap_or(0);
    if content_length > MAX_HTTP_BODY {
        bail!("HTTP body exceeds 2 MiB");
    }
    let mut body = bytes[header_end..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            bail!("connection closed before HTTP body completed");
        }
        body.extend_from_slice(&buffer[..read]);
        if body.len() > content_length {
            body.truncate(content_length);
            break;
        }
    }
    Ok(HttpRequest { method, path, body })
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        404 => "Not Found",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Response",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()?;
    Ok(())
}

pub fn parse_listen(value: &str) -> Result<SocketAddr> {
    let address: SocketAddr = value.parse().context("parse listen address")?;
    if !matches!(address.ip(), IpAddr::V4(_) | IpAddr::V6(_)) {
        bail!("invalid listen address");
    }
    Ok(address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_schema_has_exactly_one_tool_name() {
        let schema = go_input_schema();
        assert_eq!(schema["properties"]["schemaVersion"]["const"], 1);
    }

    #[test]
    fn public_bind_is_rejected_by_parser_consumer_contract() {
        let address = parse_listen("127.0.0.1:7745").unwrap();
        assert!(address.ip().is_loopback());
        let public = parse_listen("0.0.0.0:7745").unwrap();
        assert!(!public.ip().is_loopback());
    }
}
