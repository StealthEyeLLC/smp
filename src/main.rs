use std::process::ExitCode;

fn main() -> ExitCode {
    match smp::cli::run() {
        Ok(code) => ExitCode::from(u8::try_from(code.clamp(0, 255)).unwrap_or(1)),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(error.exit_code())
        }
    }
}
