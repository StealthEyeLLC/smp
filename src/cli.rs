use crate::assets;
use crate::doctor;
use crate::error::{Result, SmpError};
use crate::firecracker;
use crate::guest;
use crate::machine::{CreateOptions, Manager};
use crate::model::{MachineMode, MachineState, PortProtocol, PortPublication, Transport};
use crate::paths::Paths;
use crate::remote::Engine;
use crate::server::{self, ServeOptions};
use crate::{BUILD_COMMIT, VERSION};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::env;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(name = "smp", version = VERSION, about = "Smallest Maximum Power")]
struct Cli {
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create, start, wait for, and enter a machine.
    Up(MachineCreate),
    /// Create a machine without starting it.
    Create(MachineCreate),
    /// Start a created or stopped machine.
    Start(MachineTarget),
    /// Open a direct UID 0 SSH session.
    Ssh(MachineTarget),
    /// Execute exact argv without an implicit shell.
    Exec(ExecCommand),
    /// Copy a file using guest:/absolute/path notation.
    Cp(CopyCommand),
    /// Read bounded Firecracker output.
    Logs(LogCommand),
    /// Read bounded guest serial output.
    Console(MachineTarget),
    /// Show one machine or all machines.
    Status {
        #[arg(default_value = "default")]
        machine: String,
        #[arg(long)]
        all: bool,
    },
    /// Print the strict machine record.
    Inspect(MachineTarget),
    /// Wait for seed initialization and direct root SSH.
    Wait(MachineTarget),
    /// Gracefully stop one machine.
    Stop(MachineTarget),
    /// Kill only a verified Firecracker process.
    Kill(MachineTarget),
    /// Replace the Firecracker process while preserving machine state.
    Reboot(MachineTarget),
    /// Destroy verified machine resources.
    Destroy {
        #[arg(default_value = "default")]
        machine: String,
        #[arg(long)]
        delete_disk: bool,
    },
    /// Reconcile durable state with the host.
    Reconcile {
        #[arg(default_value = "default")]
        machine: String,
        #[arg(long)]
        all: bool,
    },
    /// Diagnose the exact x86_64/KVM/Firecracker host lane.
    Doctor {
        #[arg(long)]
        fix: bool,
    },
    /// Verify or install a freshly built canonical asset set.
    Assets {
        #[arg(long)]
        install_from: Option<PathBuf>,
    },
    /// Send an exact request to a selected machine's verified API socket.
    Api(ApiCommand),
    /// Return the live capability and filesystem contract.
    Describe {
        #[arg(long)]
        include_machines: bool,
    },
    /// Report exact SMP build identity.
    Version,
    /// Run the standalone local MCP service.
    Serve {
        #[arg(long, default_value = "/run/smp/mcp.sock")]
        socket: PathBuf,
        #[arg(long)]
        listen: Option<String>,
    },
}

#[derive(Clone, Debug, Args)]
struct MachineTarget {
    #[arg(default_value = "default")]
    machine: String,
    #[arg(long, default_value_t = 300)]
    timeout: u64,
}

#[derive(Clone, Debug, Args)]
struct MachineCreate {
    #[arg(default_value = "default")]
    machine: String,
    #[arg(long)]
    disposable: bool,
    #[arg(long)]
    mmio: bool,
    #[arg(long, default_value_t = 2)]
    vcpus: u8,
    #[arg(long, default_value_t = 1024)]
    memory_mib: u32,
    #[arg(long)]
    firecracker: Option<PathBuf>,
    #[arg(long)]
    kernel: Option<PathBuf>,
    #[arg(long)]
    rootfs: Option<PathBuf>,
    #[arg(long)]
    initrd: Option<PathBuf>,
    #[arg(long)]
    kernel_arguments: Option<String>,
    #[arg(long = "publish")]
    published_ports: Vec<String>,
    #[arg(long)]
    init: Option<PathBuf>,
    #[arg(long, default_value_t = 300)]
    timeout: u64,
}

#[derive(Clone, Debug, Args)]
struct ExecCommand {
    #[arg(long, default_value = "default")]
    machine: String,
    #[arg(long)]
    stdin: bool,
    #[arg(long)]
    tty: bool,
    #[arg(long, default_value_t = 300)]
    timeout: u64,
    #[arg(long, default_value_t = 1024 * 1024)]
    output_limit: u64,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    argv: Vec<String>,
}

#[derive(Clone, Debug, Args)]
struct CopyCommand {
    source: String,
    destination: String,
    #[arg(long, default_value = "default")]
    machine: String,
}

#[derive(Clone, Debug, Args)]
struct LogCommand {
    #[arg(default_value = "default")]
    machine: String,
    #[arg(long, default_value = "stdout")]
    stream: String,
    #[arg(long, default_value_t = 0)]
    offset: u64,
    #[arg(long, default_value_t = 64 * 1024)]
    limit: usize,
}

#[derive(Clone, Debug, Args)]
struct ApiCommand {
    #[arg(default_value = "default")]
    machine: String,
    #[arg(long, default_value = "GET")]
    method: String,
    path: String,
    #[arg(long = "header")]
    headers: Vec<String>,
    #[arg(long)]
    body_base64: Option<String>,
}

pub fn run() -> Result<i32> {
    let arguments = env::args().collect::<Vec<_>>();
    if let Some(operation) = arguments.get(1)
        && operation.starts_with("__guest-")
    {
        return guest::guest_entry(operation, arguments.get(2).map(String::as_str));
    }
    if arguments.get(1).map(String::as_str) == Some("__remote-worker") {
        let request_id = arguments
            .get(2)
            .ok_or_else(|| SmpError::Invalid("remote worker request ID is missing".to_owned()))?;
        return Engine::new(Paths::system()?).run_detached(request_id);
    }

    let cli = Cli::parse();
    let paths = Paths::system()?;
    let manager = Manager::new(paths.clone());
    match cli.command {
        Command::Up(options) => {
            assets::verify(&paths)?;
            let machine = options.machine.clone();
            if !paths.machine_record(&machine)?.exists() {
                manager.create(create_options(&options)?)?;
            }
            let current = manager.reconcile(&machine)?;
            let record = if current.state == MachineState::Ready {
                current
            } else if matches!(
                current.state,
                MachineState::Running | MachineState::Starting
            ) {
                manager.wait(&machine, Duration::from_secs(options.timeout))?
            } else {
                manager.start(&machine, Duration::from_secs(options.timeout))?
            };
            if cli.json {
                emit_json(&record)?;
                Ok(0)
            } else {
                guest::interactive_shell(&record, &manager.ssh_key())
            }
        }
        Command::Create(options) => {
            let record = manager.create(create_options(&options)?)?;
            emit_value(cli.json, &record, format!("created {}", record.machine_id))?;
            Ok(0)
        }
        Command::Start(target) => {
            let record = manager.start(&target.machine, Duration::from_secs(target.timeout))?;
            emit_value(cli.json, &record, format!("ready {}", record.machine_id))?;
            Ok(0)
        }
        Command::Ssh(target) => {
            let record = manager.wait(&target.machine, Duration::from_secs(target.timeout))?;
            if cli.json {
                return Err(SmpError::Invalid(
                    "--json is incompatible with an interactive SSH session".to_owned(),
                ));
            }
            guest::interactive_shell(&record, &manager.ssh_key())
        }
        Command::Exec(command) => execute(&manager, cli.json, command),
        Command::Cp(command) => copy(&manager, cli.json, command),
        Command::Logs(command) => logs(&manager, cli.json, command),
        Command::Console(target) => logs(
            &manager,
            cli.json,
            LogCommand {
                machine: target.machine,
                stream: "serial".to_owned(),
                offset: 0,
                limit: 64 * 1024,
            },
        ),
        Command::Status { machine, all } => {
            if all {
                let records = manager.list()?;
                emit_value(
                    cli.json,
                    &records,
                    records
                        .iter()
                        .map(|record| format!("{}\t{:?}", record.machine_id, record.state))
                        .collect::<Vec<_>>()
                        .join("\n"),
                )?;
            } else {
                let record = manager.reconcile(&machine)?;
                emit_value(
                    cli.json,
                    &record,
                    format!("{}\t{:?}", record.machine_id, record.state),
                )?;
            }
            Ok(0)
        }
        Command::Inspect(target) => {
            let record = manager.load(&target.machine)?;
            emit_json(&record)?;
            Ok(0)
        }
        Command::Wait(target) => {
            let record = manager.wait(&target.machine, Duration::from_secs(target.timeout))?;
            emit_value(cli.json, &record, format!("ready {}", record.machine_id))?;
            Ok(0)
        }
        Command::Stop(target) => {
            let record = manager.stop(&target.machine, Duration::from_secs(target.timeout))?;
            emit_value(cli.json, &record, format!("stopped {}", record.machine_id))?;
            Ok(0)
        }
        Command::Kill(target) => {
            let record = manager.kill(&target.machine)?;
            emit_value(cli.json, &record, format!("killed {}", record.machine_id))?;
            Ok(0)
        }
        Command::Reboot(target) => {
            let result = manager.reboot(&target.machine, Duration::from_secs(target.timeout))?;
            emit_value(
                cli.json,
                &result,
                format!(
                    "rebooted {} -> {}",
                    result.old_process.pid, result.new_process.pid
                ),
            )?;
            Ok(0)
        }
        Command::Destroy {
            machine,
            delete_disk,
        } => {
            manager.destroy(&machine, delete_disk)?;
            emit_value(
                cli.json,
                &json!({"destroyed": machine}),
                "destroyed".to_owned(),
            )?;
            Ok(0)
        }
        Command::Reconcile { machine, all } => {
            if all {
                let records = Engine::new(paths).reconcile_all()?;
                emit_value(cli.json, &records, format!("reconciled {}", records.len()))?;
            } else {
                let record = manager.reconcile(&machine)?;
                emit_value(
                    cli.json,
                    &record,
                    format!("{} {:?}", record.machine_id, record.state),
                )?;
            }
            Ok(0)
        }
        Command::Doctor { fix } => {
            let changes = if fix {
                doctor::fix(&paths)?
            } else {
                Vec::new()
            };
            let report = doctor::inspect(&paths);
            let healthy = report.healthy;
            let human = report
                .checks
                .iter()
                .map(|check| {
                    format!(
                        "{} {}: {}",
                        if check.ok { "ok" } else { "fail" },
                        check.name,
                        check.detail
                    )
                })
                .chain(changes.iter().map(|change| format!("changed: {change}")))
                .collect::<Vec<_>>()
                .join("\n");
            emit_value(
                cli.json,
                &json!({"changed": changes, "report": report}),
                human,
            )?;
            Ok(if healthy { 0 } else { 20 })
        }
        Command::Assets { install_from } => {
            if let Some(build) = install_from {
                assets::install_from_build(&paths, &build)?;
            }
            let verified = assets::verify(&paths)?;
            emit_value(
                cli.json,
                &verified.manifest,
                format!("verified assets {}", verified.manifest_digest),
            )?;
            Ok(0)
        }
        Command::Api(command) => api(&manager, cli.json, command),
        Command::Describe { include_machines } => {
            let value = Engine::new(paths).describe(include_machines)?;
            emit_value(
                cli.json,
                &value,
                serde_json::to_string_pretty(&value)
                    .map_err(|error| SmpError::json("<stdout>", error))?,
            )?;
            Ok(0)
        }
        Command::Version => {
            emit_value(
                cli.json,
                &json!({"product": "smp", "version": VERSION, "buildCommit": BUILD_COMMIT}),
                format!("smp {VERSION} ({BUILD_COMMIT})"),
            )?;
            Ok(0)
        }
        Command::Serve { socket, listen } => {
            server::serve(paths, ServeOptions { socket, listen })?;
            Ok(0)
        }
    }
}

fn execute(manager: &Manager, json_output: bool, command: ExecCommand) -> Result<i32> {
    let record = manager.load(&command.machine)?;
    let mut stdin = Vec::new();
    let input = if command.stdin {
        io::stdin()
            .lock()
            .read_to_end(&mut stdin)
            .map_err(|error| SmpError::io("<stdin>", error))?;
        Some(stdin.as_slice())
    } else {
        None
    };
    let result = guest::execute(
        &record,
        &manager.ssh_key(),
        &command.argv,
        input,
        Duration::from_secs(command.timeout),
        command.output_limit,
        command.tty,
    )?;
    if json_output {
        emit_json(&json!({
            "exitCode": result.exit_code,
            "signal": result.signal,
            "timedOut": result.timed_out,
            "stdoutBase64": STANDARD.encode(&result.stdout),
            "stderrBase64": STANDARD.encode(&result.stderr),
            "stdoutComplete": result.stdout_complete,
            "stderrComplete": result.stderr_complete,
            "totalStdoutBytes": result.total_stdout_bytes,
            "totalStderrBytes": result.total_stderr_bytes
        }))?;
    } else {
        io::stdout()
            .lock()
            .write_all(&result.stdout)
            .map_err(|error| SmpError::io("<stdout>", error))?;
        io::stderr()
            .lock()
            .write_all(&result.stderr)
            .map_err(|error| SmpError::io("<stderr>", error))?;
    }
    Ok(result.exit_code)
}

fn copy(manager: &Manager, json_output: bool, command: CopyCommand) -> Result<i32> {
    let record = manager.load(&command.machine)?;
    let key = manager.ssh_key();
    match (
        guest_path(&command.source),
        guest_path(&command.destination),
    ) {
        (None, Some(destination)) => {
            guest::upload(&record, &key, Path::new(&command.source), &destination)?;
            emit_value(
                json_output,
                &json!({"direction":"upload","destination":destination}),
                "uploaded".to_owned(),
            )?;
        }
        (Some(source), None) => {
            let destination = PathBuf::from(&command.destination);
            guest::download(&record, &key, &source, &destination)?;
            emit_value(
                json_output,
                &json!({"direction":"download","source":source,"destination":destination}),
                "downloaded".to_owned(),
            )?;
        }
        _ => {
            return Err(SmpError::Invalid(
                "exactly one cp operand must use guest:/absolute/path".to_owned(),
            ));
        }
    }
    Ok(0)
}

fn logs(manager: &Manager, json_output: bool, command: LogCommand) -> Result<i32> {
    let chunk = manager.read_log(
        &command.machine,
        &command.stream,
        command.offset,
        command.limit,
    )?;
    if json_output {
        emit_json(&json!({
            "path": chunk.path,
            "offset": chunk.offset,
            "nextOffset": chunk.next_offset,
            "dataBase64": STANDARD.encode(&chunk.data),
            "eof": chunk.eof,
            "truncated": chunk.truncated
        }))?;
    } else {
        io::stdout()
            .lock()
            .write_all(&chunk.data)
            .map_err(|error| SmpError::io("<stdout>", error))?;
    }
    Ok(0)
}

fn api(manager: &Manager, json_output: bool, command: ApiCommand) -> Result<i32> {
    let record = manager.load(&command.machine)?;
    let mut headers = BTreeMap::new();
    for header in command.headers {
        let (name, value) = header
            .split_once(':')
            .ok_or_else(|| SmpError::Invalid(format!("invalid header {header}")))?;
        headers.insert(name.trim().to_owned(), value.trim().to_owned());
    }
    let body = command
        .body_base64
        .as_deref()
        .map(|value| {
            STANDARD
                .decode(value)
                .map_err(|error| SmpError::Invalid(format!("invalid body base64: {error}")))
        })
        .transpose()?
        .unwrap_or_default();
    let response = firecracker::raw_api(
        &record,
        &manager.paths.runtime,
        &command.method,
        &command.path,
        &headers,
        &body,
    )?;
    let value = json!({
        "statusCode": response.status_code,
        "headers": response.headers,
        "bodyBase64": STANDARD.encode(response.body)
    });
    emit_value(
        json_output,
        &value,
        serde_json::to_string_pretty(&value).map_err(|error| SmpError::json("<stdout>", error))?,
    )?;
    Ok(if (200..300).contains(&response.status_code) {
        0
    } else {
        22
    })
}

fn create_options(options: &MachineCreate) -> Result<CreateOptions> {
    Ok(CreateOptions {
        machine_id: options.machine.clone(),
        mode: if options.disposable {
            MachineMode::Disposable
        } else {
            MachineMode::Persistent
        },
        transport: if options.mmio {
            Transport::Mmio
        } else {
            Transport::Pci
        },
        vcpu_count: options.vcpus,
        memory_mib: options.memory_mib,
        firecracker: options.firecracker.clone(),
        kernel: options.kernel.clone(),
        rootfs: options.rootfs.clone(),
        initrd: options.initrd.clone(),
        kernel_arguments: options.kernel_arguments.clone(),
        published_ports: options
            .published_ports
            .iter()
            .map(|value| parse_publication(value))
            .collect::<Result<Vec<_>>>()?,
        initialization_script: options.init.clone(),
    })
}

fn parse_publication(value: &str) -> Result<PortPublication> {
    let parts = value.split(':').collect::<Vec<_>>();
    let (protocol, bind, host, guest) = match parts.as_slice() {
        [protocol, host, guest] => (*protocol, "0.0.0.0", *host, *guest),
        [protocol, bind, host, guest] => (*protocol, *bind, *host, *guest),
        _ => {
            return Err(SmpError::Invalid(format!(
                "publication must be PROTOCOL[:BIND]:HOST:GUEST: {value}"
            )));
        }
    };
    Ok(PortPublication {
        protocol: match protocol {
            "tcp" => PortProtocol::Tcp,
            "udp" => PortProtocol::Udp,
            _ => {
                return Err(SmpError::Invalid(format!(
                    "unsupported protocol {protocol}"
                )));
            }
        },
        bind_address: bind.to_owned(),
        host_port: host
            .parse()
            .map_err(|_| SmpError::Invalid(format!("invalid host port {host}")))?,
        guest_port: guest
            .parse()
            .map_err(|_| SmpError::Invalid(format!("invalid guest port {guest}")))?,
    })
}

fn guest_path(value: &str) -> Option<PathBuf> {
    value.strip_prefix("guest:").map(PathBuf::from)
}

fn emit_json<T: Serialize>(value: &T) -> Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, value)
        .map_err(|error| SmpError::json("<stdout>", error))?;
    stdout
        .write_all(b"\n")
        .map_err(|error| SmpError::io("<stdout>", error))
}

fn emit_value<T: Serialize>(json_output: bool, value: &T, human: String) -> Result<()> {
    if json_output {
        emit_json(value)
    } else {
        writeln!(io::stdout().lock(), "{human}").map_err(|error| SmpError::io("<stdout>", error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_parser_rejects_unknown_protocol() {
        assert!(parse_publication("sctp:123:456").is_err());
        assert!(parse_publication("tcp:127.0.0.1:123:456").is_ok());
    }

    #[test]
    fn guest_copy_notation_is_explicit() {
        assert_eq!(guest_path("guest:/root/x"), Some(PathBuf::from("/root/x")));
        assert_eq!(guest_path("/tmp/x"), None);
    }
}
