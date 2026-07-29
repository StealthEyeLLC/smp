use crate::doctor;
use crate::error::{Result, SmpError};
use crate::paths::Paths;
use crate::{
    BUILD_COMMIT, MACHINE_SCHEMA_VERSION, REQUEST_SCHEMA_VERSION, RESPONSE_SCHEMA_VERSION, VERSION,
};
use clap::{Parser, Subcommand};
use serde::Serialize;
use serde_json::json;
use std::io::{self, Write};

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
    /// Report exact SMP build identity.
    Version,
    /// Return the live capability and filesystem contract.
    Describe {
        #[arg(long)]
        include_machines: bool,
    },
    /// Diagnose the exact x86_64/KVM/Firecracker host lane.
    Doctor {
        #[arg(long)]
        fix: bool,
    },
}

pub fn run() -> Result<i32> {
    let cli = Cli::parse();
    let paths = Paths::system()?;
    match cli.command {
        Command::Version => {
            emit(
                cli.json,
                &json!({
                    "product": "smp",
                    "version": VERSION,
                    "buildCommit": BUILD_COMMIT
                }),
                format!("smp {VERSION} ({BUILD_COMMIT})"),
            )?;
            Ok(0)
        }
        Command::Describe { include_machines } => {
            let value = json!({
                "product": "SMP",
                "executable": "smp",
                "version": VERSION,
                "buildCommit": BUILD_COMMIT,
                "machineSchemaVersion": MACHINE_SCHEMA_VERSION,
                "requestSchemaVersion": REQUEST_SCHEMA_VERSION,
                "responseSchemaVersion": RESPONSE_SCHEMA_VERSION,
                "architecture": std::env::consts::ARCH,
                "defaultMachine": "default",
                "plugin": {
                    "displayName": "SMP",
                    "namespace": "smp",
                    "onlyTool": "go",
                    "callableIdentity": "smp.go"
                },
                "operations": ["describe", "doctor"],
                "limits": {
                    "inlineOutputBytes": 1048576,
                    "totalCaptureBytes": 67108864,
                    "timeoutSeconds": 86400,
                    "requestRetentionSeconds": 86400,
                    "resultRetentionSeconds": 86400
                },
                "directories": paths.directory_contract(),
                "machines": if include_machines { json!([]) } else { serde_json::Value::Null }
            });
            emit(
                cli.json,
                &value,
                serde_json::to_string_pretty(&value)
                    .map_err(|error| SmpError::json("<stdout>", error))?,
            )?;
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
            let checks = report
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
                .collect::<Vec<_>>()
                .join("\n");
            let human = if changes.is_empty() {
                checks
            } else {
                format!("changed: {}\n{checks}", changes.join("; "))
            };
            if cli.json {
                emit(
                    true,
                    &json!({"changed": changes, "report": report}),
                    String::new(),
                )?;
            } else {
                emit(false, &report, human)?;
            }
            Ok(if healthy { 0 } else { 20 })
        }
    }
}

fn emit<T: Serialize>(json_output: bool, value: &T, human: String) -> Result<()> {
    let mut stdout = io::stdout().lock();
    if json_output {
        serde_json::to_writer_pretty(&mut stdout, value)
            .map_err(|error| SmpError::json("<stdout>", error))?;
        stdout
            .write_all(b"\n")
            .map_err(|error| SmpError::io("<stdout>", error))?;
    } else {
        writeln!(stdout, "{human}").map_err(|error| SmpError::io("<stdout>", error))?;
    }
    Ok(())
}
