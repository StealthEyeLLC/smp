use crate::model::{
    AssetIdentity, DEBIAN_SUITE, DEBIAN_VERSION, FIRECRACKER_VERSION, KERNEL_VERSION,
};
use crate::state::RuntimePaths;
use crate::util::{read_json, sha256_file};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AssetManifest {
    pub schema_version: u32,
    pub architecture: String,
    pub firecracker: AssetIdentity,
    pub kernel: AssetIdentity,
    pub rootfs: AssetIdentity,
    pub module_tree_sha256: String,
    pub firecracker_release_asset: String,
    pub firecracker_release_sha256: String,
    pub kernel_source_url: String,
    pub kernel_config_sha256: String,
    pub debian_suite: String,
    pub debian_version: String,
    pub debian_mirror: String,
    pub debian_inrelease_sha256: String,
    pub package_manifest_path: String,
    pub built_at: String,
}

impl AssetManifest {
    pub fn verify(&self) -> Result<()> {
        if self.schema_version != 1 {
            bail!("unsupported asset manifest schema {}", self.schema_version);
        }
        if self.architecture != "x86_64" {
            bail!("unsupported asset architecture {}", self.architecture);
        }
        if self.firecracker.version != FIRECRACKER_VERSION {
            bail!(
                "expected Firecracker {FIRECRACKER_VERSION}, found {}",
                self.firecracker.version
            );
        }
        if self.kernel.version != KERNEL_VERSION {
            bail!(
                "expected Linux {KERNEL_VERSION}, found {}",
                self.kernel.version
            );
        }
        if self.debian_suite != DEBIAN_SUITE || self.debian_version != DEBIAN_VERSION {
            bail!(
                "expected Debian {DEBIAN_VERSION} {DEBIAN_SUITE}, found {} {}",
                self.debian_version,
                self.debian_suite
            );
        }
        for (label, identity) in [
            ("Firecracker", &self.firecracker),
            ("kernel", &self.kernel),
            ("rootfs", &self.rootfs),
        ] {
            let path = PathBuf::from(&identity.path);
            if !path.is_file() {
                bail!("{label} asset is missing: {}", path.display());
            }
            let observed = sha256_file(&path)?;
            if observed != identity.sha256 {
                bail!(
                    "{label} digest mismatch for {}: expected {}, observed {}",
                    path.display(),
                    identity.sha256,
                    observed
                );
            }
        }
        Ok(())
    }
}

pub fn manifest_path(paths: &RuntimePaths) -> PathBuf {
    paths.assets_root().join("manifest.json")
}

pub fn load_manifest(paths: &RuntimePaths) -> Result<AssetManifest> {
    let path = manifest_path(paths);
    let manifest: AssetManifest = read_json(&path)
        .with_context(|| format!("load SMP asset manifest from {}", path.display()))?;
    manifest.verify()?;
    Ok(manifest)
}

pub fn ensure_assets(paths: &RuntimePaths, offline: bool) -> Result<AssetManifest> {
    if let Ok(manifest) = load_manifest(paths) {
        return Ok(manifest);
    }
    let script = paths.lib_root.join("build-assets.sh");
    if !script.is_file() {
        bail!(
            "verified assets are unavailable and the SMP-owned builder is missing: {}",
            script.display()
        );
    }
    let mut command = Command::new(&script);
    command
        .arg("--assets-root")
        .arg(paths.assets_root())
        .arg("--etc-root")
        .arg(&paths.etc_root);
    if offline {
        command.arg("--offline");
    }
    let status = command
        .status()
        .with_context(|| format!("run {}", script.display()))?;
    if !status.success() {
        bail!("SMP asset preparation failed with {status}");
    }
    load_manifest(paths)
}

pub fn describe_manifest(paths: &RuntimePaths) -> serde_json::Value {
    match load_manifest(paths) {
        Ok(manifest) => serde_json::to_value(manifest).unwrap_or(serde_json::Value::Null),
        Err(error) => serde_json::json!({
            "available": false,
            "error": error.to_string(),
            "expected": {
                "firecracker": FIRECRACKER_VERSION,
                "kernel": KERNEL_VERSION,
                "debianSuite": DEBIAN_SUITE,
                "debianVersion": DEBIAN_VERSION
            }
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_rejects_newer_schema() {
        let manifest = AssetManifest {
            schema_version: 2,
            architecture: "x86_64".to_owned(),
            firecracker: AssetIdentity {
                path: "/missing".to_owned(),
                sha256: String::new(),
                version: FIRECRACKER_VERSION.to_owned(),
                provenance_path: None,
            },
            kernel: AssetIdentity {
                path: "/missing".to_owned(),
                sha256: String::new(),
                version: KERNEL_VERSION.to_owned(),
                provenance_path: None,
            },
            rootfs: AssetIdentity {
                path: "/missing".to_owned(),
                sha256: String::new(),
                version: DEBIAN_VERSION.to_owned(),
                provenance_path: None,
            },
            module_tree_sha256: String::new(),
            firecracker_release_asset: String::new(),
            firecracker_release_sha256: String::new(),
            kernel_source_url: String::new(),
            kernel_config_sha256: String::new(),
            debian_suite: DEBIAN_SUITE.to_owned(),
            debian_version: DEBIAN_VERSION.to_owned(),
            debian_mirror: String::new(),
            debian_inrelease_sha256: String::new(),
            package_manifest_path: String::new(),
            built_at: String::new(),
        };
        assert!(manifest.verify().is_err());
    }
}
