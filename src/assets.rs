use crate::error::{Result, SmpError};
use crate::paths::Paths;
use crate::util::{read_json, sha256_file};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssetManifest {
    pub schema_version: u32,
    pub product: String,
    pub architecture: String,
    pub firecracker: FirecrackerAsset,
    pub kernel: KernelAsset,
    pub debian: DebianAsset,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FirecrackerAsset {
    pub version: String,
    pub architecture: String,
    pub source_url: String,
    pub archive_sha256: String,
    pub binary_path: String,
    pub binary_sha256: String,
    pub version_output: String,
    pub build_timestamp: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KernelAsset {
    pub version: String,
    pub source_date: String,
    pub source_url: String,
    pub source_sha256: String,
    pub config_path: String,
    pub config_sha256: String,
    pub vmlinux_path: String,
    pub vmlinux_sha256: String,
    pub module_tree_sha256: String,
    pub modules_archive_path: String,
    pub modules_archive_sha256: String,
    pub build_timestamp: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DebianArchiveKeyringAsset {
    pub version: String,
    pub source_url: String,
    pub package_sha256: String,
    pub keyring_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DebianBaseFilesAsset {
    pub version: String,
    pub source_url: String,
    pub package_sha256: String,
    pub debian_version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DebianAsset {
    pub version: String,
    pub suite: String,
    pub architecture: String,
    pub requested_snapshot_timestamp: String,
    pub snapshot_timestamp: String,
    pub snapshot_correction_reason: String,
    pub archive_keyring: DebianArchiveKeyringAsset,
    pub base_files: DebianBaseFilesAsset,
    pub repositories: Vec<String>,
    pub in_release_sha256: Vec<String>,
    pub package_list_path: String,
    pub package_list_sha256: String,
    pub filesystem_uuid: String,
    pub filesystem_size: u64,
    pub rootfs_path: String,
    pub rootfs_sha256: String,
    pub seed_template_path: String,
    pub seed_template_uuid: String,
    pub seed_template_sha256: String,
    pub guest_helper_sha256: String,
    pub module_tree_sha256: String,
    pub build_timestamp: String,
}

#[derive(Clone, Debug)]
pub struct VerifiedAssets {
    pub manifest: AssetManifest,
    pub manifest_digest: String,
    pub firecracker: PathBuf,
    pub kernel: PathBuf,
    pub rootfs: PathBuf,
    pub seed_template: PathBuf,
}

pub fn verify(paths: &Paths) -> Result<VerifiedAssets> {
    let manifest_path = paths.assets.join("manifest.json");
    let manifest: AssetManifest = read_json(&manifest_path)?;
    if manifest.schema_version != 1
        || manifest.product != "SMP"
        || manifest.architecture != "x86_64"
        || manifest.firecracker.version != "1.15.1"
        || manifest.kernel.version != "6.1.178"
        || manifest.debian.version != "13.6"
        || manifest.debian.suite != "trixie"
        || manifest.debian.requested_snapshot_timestamp != "20260711T000000Z"
        || manifest.debian.snapshot_timestamp != "20260711T103542Z"
        || manifest.debian.snapshot_correction_reason.is_empty()
        || manifest.debian.archive_keyring.version != "2025.1"
        || manifest.debian.base_files.version != "13.8+deb13u6"
        || manifest.debian.base_files.debian_version != "13.6"
    {
        return Err(SmpError::State(
            "asset manifest does not match the canonical baseline".to_owned(),
        ));
    }
    let firecracker = confined_asset(
        &paths.assets,
        &manifest.firecracker.binary_path,
        &manifest.firecracker.binary_sha256,
    )?;
    let kernel = confined_asset(
        &paths.assets,
        &manifest.kernel.vmlinux_path,
        &manifest.kernel.vmlinux_sha256,
    )?;
    let rootfs = confined_asset(
        &paths.assets,
        &manifest.debian.rootfs_path,
        &manifest.debian.rootfs_sha256,
    )?;
    let seed_template = confined_asset(
        &paths.assets,
        &manifest.debian.seed_template_path,
        &manifest.debian.seed_template_sha256,
    )?;
    let version = Command::new(&firecracker)
        .arg("--version")
        .output()
        .map_err(|error| SmpError::io(firecracker.display().to_string(), error))?;
    if !version.status.success()
        || !String::from_utf8_lossy(&version.stdout).contains("Firecracker v1.15.1")
    {
        return Err(SmpError::State(
            "verified Firecracker binary did not report v1.15.1".to_owned(),
        ));
    }
    Ok(VerifiedAssets {
        manifest_digest: sha256_file(&manifest_path)?,
        manifest,
        firecracker,
        kernel,
        rootfs,
        seed_template,
    })
}

pub fn install_from_build(paths: &Paths, build_output: &Path) -> Result<()> {
    paths.ensure_layout()?;
    for entry in fs::read_dir(build_output)
        .map_err(|error| SmpError::io(build_output.display().to_string(), error))?
    {
        let entry =
            entry.map_err(|error| SmpError::io(build_output.display().to_string(), error))?;
        let source = entry.path();
        if !entry
            .file_type()
            .map_err(|error| SmpError::io(source.display().to_string(), error))?
            .is_file()
        {
            continue;
        }
        let destination = paths.assets.join(entry.file_name());
        fs::copy(&source, &destination)
            .map_err(|error| SmpError::io(destination.display().to_string(), error))?;
    }
    verify(paths).map(|_| ())
}

fn confined_asset(root: &Path, relative: &str, expected_digest: &str) -> Result<PathBuf> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(SmpError::Invalid(format!(
            "unsafe asset path {}",
            relative.display()
        )));
    }
    let path = root.join(relative);
    let digest = sha256_file(&path)?;
    if digest != expected_digest {
        return Err(SmpError::State(format!(
            "asset digest mismatch for {}: expected {}, got {}",
            path.display(),
            expected_digest,
            digest
        )));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_paths_reject_traversal() {
        assert!(confined_asset(Path::new("/var/lib/smp/assets"), "../etc/passwd", "x").is_err());
        assert!(confined_asset(Path::new("/var/lib/smp/assets"), "/etc/passwd", "x").is_err());
    }
}
