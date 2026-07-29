use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=SMP_BUILD_COMMIT");
    let git_dir = git_directory();
    let head = git_dir.join("HEAD");
    println!("cargo:rerun-if-changed={}", head.display());
    println!(
        "cargo:rerun-if-changed={}",
        git_dir.join("packed-refs").display()
    );
    if let Ok(contents) = fs::read_to_string(&head)
        && let Some(reference) = contents.trim().strip_prefix("ref: ")
    {
        println!(
            "cargo:rerun-if-changed={}",
            git_dir.join(reference).display()
        );
    }
    let commit = std::env::var("SMP_BUILD_COMMIT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .map(|value| value.trim().to_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=SMP_BUILD_COMMIT={commit}");
}

fn git_directory() -> PathBuf {
    let dot_git = PathBuf::from(".git");
    if dot_git.is_dir() {
        return dot_git;
    }
    fs::read_to_string(&dot_git)
        .ok()
        .and_then(|contents| contents.trim().strip_prefix("gitdir: ").map(PathBuf::from))
        .unwrap_or(dot_git)
}
