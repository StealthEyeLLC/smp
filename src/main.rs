use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::json;
use smp::core::{self, CreateOptions};
use smp::model::{MachineMode, PublishedPort, VirtioTransport, BUILD_COMMIT, SMP_VERSION};
use smp::remote;
use smp::server;
use smp::state::RuntimePaths;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(name = "smp", version = SMP_VERSION, about = "Smallest Maximum Power Firecracker microVM controller")]
struct Cli {
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true, hide = true)]
    state_root: Option<PathBuf>,
    #[arg(long, global = true, hide = true)]
    etc_root: Option<PathBuf>,
    #[arg(long, global = true, hide = true)]
    run_root: Option<PathBuf>,
    #[arg(long, global = true, hide = true)]
    lib_root: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Up(CreateArgs),
    Create(CreateArgs),
    Start {
        #[arg(default_value = "default")]
        name: String,
        #[arg(long)]
        foreground: bool,
    },
    Ssh {
        #[arg(default_value = "default")]
        name: String,
    },
    Exec {
        #[arg(default_value = "default")]
        name: String,
        #[arg(long)]
        tty: bool,
        #[arg(last = true, required = true)]
        argv: Vec<String>,
    },
    Cp {
        #[arg(default_value = "default")]
        name: String,
        source: String,
        destination: String,
    },
    Logs {
        #[arg(default_value = "default")]
        name: String,
        #[arg(long)]
        follow: bool,
        #[arg(long, default_value_t = 200)]
        lines: u64,
    },
    Console {
        #[arg(default_value = "default")]
        name: String,
    },
    Status {
        #[arg(default_value = "default")]
        name: String,
    },
    Inspect {
        #[arg(default_value = "default")]
        name: String,
    },
    Wait {
        #[arg(default_value = "default")]
        name: String,
        #[arg(long, default_value_t = 120)]
        timeout_seconds: u64,
    },
    Stop {
        #[arg(default_value = "default")]
        name: String,
    },
    Kill {
        #[arg(default_value = "default")]
        name: String,
    },
    Reboot {
        #[arg(default_value = "default")]
        name: String,
    },
    Destroy {
        #[arg(default_value = "default")]
        name: String,
        #[arg(long)]
        force: bool,
    },
    Reconcile {
        #[arg(default_value = "default")]
        name: String,
    },
    Doctor {
        #[arg(long)]
        fix: bool,
    },
    Api {
        #[arg(default_value = "default")]
        name: String,
        #[arg(long)]
        method: String,
        #[arg(long)]
        path: String,
        #[arg(long = "header")]
        headers: Vec<String>,
        #[arg(long)]
        body: Option<String>,
        #[arg(long)]
        body_base64: Option<String>,
    },
    Describe {
        #[arg(long)]
        machines: bool,
    },
    Version,
    Serve {
        #[arg(long, default_value = "127.0.0.1:7745")]
        listen: String,
    },
    Assets {
        #[arg(long)]
        offline: bool,
    },
    #[command(name = "__detached-worker", hide = true)]
    DetachedWorker {
        #[arg(long)]
        handle: String,
    },
}

#[derive(Debug, Clone, Args)]
struct CreateArgs {
    #[arg(default_value = "default")]
    name: String,
    #[arg(long, value_enum, default_value_t = ModeArg::Persistent)]
    mode: ModeArg,
    #[arg(long, value_enum, default_value_t = TransportArg::Pci)]
    transport: TransportArg,
    #[arg(long, default_value_t = 2)]
    vcpus: u8,
    #[arg(long, default_value_t = 2048)]
    memory_mib: u32,
    #[arg(long)]
    rootfs: Option<PathBuf>,
    #[arg(long)]
    kernel: Option<PathBuf>,
    #[arg(long)]
    firecracker: Option<PathBuf>,
    #[arg(long)]
    boot_args: Option<String>,
    #[arg(long = "publish", value_parser = parse_port)]
    published_ports: Vec<PublishedPort>,
    #[arg(long)]
    offline: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ModeArg {
    Persistent,
    Disposable,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TransportArg {
    Pci,
    Mmio,
}

fn main() {
    let cli = Cli::parse();
    let paths = paths_from_cli(&cli);
    match run(&cli, &paths) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            if cli.json {
                println!(
                    "{}",
                    json!({"ok": false, "error": {"code": "SMP_COMMAND_FAILED", "message": error.to_string()}})
                );
            } else {
                eprintln!("smp: {error:#}");
            }
            std::process::exit(1);
        }
    }
}

fn run(cli: &Cli, paths: &RuntimePaths) -> Result<i32> {
    match &cli.command {
        Command::Up(args) => core::up(paths, &create_options(args)),
        Command::Create(args) => {
            print_value(cli.json, &core::create(paths, &create_options(args))?)?;
            Ok(0)
        }
        Command::Start { name, foreground } => {
            print_value(cli.json, &core::start(paths, name, *foreground)?)?;
            Ok(0)
        }
        Command::Ssh { name } => core::ssh(paths, name),
        Command::Exec { name, tty, argv } => core::exec(paths, name, argv, *tty),
        Command::Cp {
            name,
            source,
            destination,
        } => {
            core::copy(paths, name, source, destination)?;
            print_value(
                cli.json,
                &json!({"copied": true, "source": source, "destination": destination}),
            )?;
            Ok(0)
        }
        Command::Logs {
            name,
            follow,
            lines,
        } => core::logs(paths, name, *follow, *lines),
        Command::Console { name } => core::console(paths, name),
        Command::Status { name } | Command::Inspect { name } => {
            print_value(cli.json, &core::status(paths, name)?)?;
            Ok(0)
        }
        Command::Wait {
            name,
            timeout_seconds,
        } => {
            print_value(
                cli.json,
                &core::wait(paths, name, Duration::from_secs(*timeout_seconds))?,
            )?;
            Ok(0)
        }
        Command::Stop { name } => {
            print_value(cli.json, &core::stop(paths, name)?)?;
            Ok(0)
        }
        Command::Kill { name } => {
            print_value(cli.json, &core::kill(paths, name)?)?;
            Ok(0)
        }
        Command::Reboot { name } => {
            let (old, machine) = core::reboot(paths, name)?;
            print_value(
                cli.json,
                &json!({"oldProcess": old, "newProcess": machine.process, "machine": machine}),
            )?;
            Ok(0)
        }
        Command::Destroy { name, force } => {
            core::destroy(paths, name, *force)?;
            print_value(cli.json, &json!({"destroyed": name}))?;
            Ok(0)
        }
        Command::Reconcile { name } => {
            print_value(cli.json, &core::reconcile(paths, name)?)?;
            Ok(0)
        }
        Command::Doctor { fix } => {
            let report = smp::doctor::run_doctor(paths, *fix)?;
            let healthy = report.healthy;
            print_value(cli.json, &report)?;
            Ok(if healthy { 0 } else { 2 })
        }
        Command::Api {
            name,
            method,
            path,
            headers,
            body,
            body_base64,
        } => {
            if body.is_some() && body_base64.is_some() {
                bail!("use only one of --body or --body-base64");
            }
            let headers = parse_headers(headers)?;
            let body = match body_base64 {
                Some(value) => BASE64.decode(value).context("decode --body-base64")?,
                None => body.clone().unwrap_or_default().into_bytes(),
            };
            let (status, response) = core::api(paths, name, method, path, &headers, &body)?;
            if cli.json {
                print_value(
                    true,
                    &json!({"httpStatus": status, "bodyBase64": BASE64.encode(response)}),
                )?;
            } else {
                println!("HTTP {status}");
                std::io::Write::write_all(&mut std::io::stdout(), &response)?;
            }
            Ok(if (200..300).contains(&status) { 0 } else { 1 })
        }
        Command::Describe { machines } => {
            print_value(true, &remote::describe(paths, *machines)?)?;
            Ok(0)
        }
        Command::Version => {
            print_value(
                cli.json,
                &json!({"version": SMP_VERSION, "buildCommit": BUILD_COMMIT}),
            )?;
            Ok(0)
        }
        Command::Serve { listen } => {
            server::serve(paths.clone(), server::parse_listen(listen)?)?;
            Ok(0)
        }
        Command::Assets { offline } => {
            print_value(cli.json, &smp::assets::ensure_assets(paths, *offline)?)?;
            Ok(0)
        }
        Command::DetachedWorker { handle } => {
            remote::run_detached_worker(paths, handle)?;
            Ok(0)
        }
    }
}

fn paths_from_cli(cli: &Cli) -> RuntimePaths {
    let mut paths = RuntimePaths::default();
    if let Some(value) = &cli.state_root {
        paths.state_root = value.clone();
    }
    if let Some(value) = &cli.etc_root {
        paths.etc_root = value.clone();
    }
    if let Some(value) = &cli.run_root {
        paths.run_root = value.clone();
    }
    if let Some(value) = &cli.lib_root {
        paths.lib_root = value.clone();
    }
    paths
}

fn create_options(args: &CreateArgs) -> CreateOptions {
    CreateOptions {
        name: args.name.clone(),
        mode: match args.mode {
            ModeArg::Persistent => MachineMode::Persistent,
            ModeArg::Disposable => MachineMode::Disposable,
        },
        transport: match args.transport {
            TransportArg::Pci => VirtioTransport::Pci,
            TransportArg::Mmio => VirtioTransport::Mmio,
        },
        vcpu_count: args.vcpus,
        memory_mib: args.memory_mib,
        rootfs: args.rootfs.clone(),
        kernel: args.kernel.clone(),
        firecracker: args.firecracker.clone(),
        boot_args: args.boot_args.clone(),
        published_ports: args.published_ports.clone(),
        offline: args.offline,
    }
}

fn parse_port(value: &str) -> Result<PublishedPort, String> {
    let fields: Vec<&str> = value.split(':').collect();
    let (protocol, host, guest) = match fields.as_slice() {
        [host, guest] => ("tcp", *host, *guest),
        [protocol, host, guest] => (*protocol, *host, *guest),
        _ => return Err("publish must be HOST:GUEST or PROTOCOL:HOST:GUEST".to_owned()),
    };
    if !matches!(protocol, "tcp" | "udp") {
        return Err("publish protocol must be tcp or udp".to_owned());
    }
    Ok(PublishedPort {
        protocol: protocol.to_owned(),
        host_port: host.parse().map_err(|_| "invalid host port".to_owned())?,
        guest_port: guest.parse().map_err(|_| "invalid guest port".to_owned())?,
    })
}

fn parse_headers(values: &[String]) -> Result<Vec<(String, String)>> {
    values
        .iter()
        .map(|value| {
            let (name, value) = value
                .split_once(':')
                .ok_or_else(|| anyhow::anyhow!("header must be NAME:VALUE"))?;
            Ok((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

fn print_value<T: Serialize>(json_mode: bool, value: &T) -> Result<()> {
    if json_mode {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        let value = serde_json::to_value(value)?;
        match value {
            serde_json::Value::String(value) => println!("{value}"),
            value => println!("{}", serde_json::to_string_pretty(&value)?),
        }
    }
    Ok(())
}
