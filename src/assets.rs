use crate::model::{
    AssetIdentity, DEBIAN_SUITE, DEBIAN_VERSION, FIRECRACKER_VERSION, KERNEL_VERSION,
};
use crate::state::RuntimePaths;
use crate::util::{read_json, sha256_file};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const MODULE_TREE_DIGEST_ALGORITHM: &str = "sha256-relative-regular-files-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AssetManifest {
    pub schema_version: u32,
    pub architecture: String,
    pub firecracker: AssetIdentity,
    pub kernel: AssetIdentity,
    pub rootfs: AssetIdentity,
    pub module_tree_sha256: String,
    pub module_tree_digest_algorithm: String,
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
        if self.module_tree_digest_algorithm != MODULE_TREE_DIGEST_ALGORITHM {
            bail!(
                "unsupported module-tree digest algorithm {}",
                self.module_tree_digest_algorithm
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
        let kernel_path = PathBuf::from(&self.kernel.path);
        let kernel_parent = kernel_path.parent().ok_or_else(|| {
            anyhow::anyhow!("kernel path has no parent: {}", kernel_path.display())
        })?;
        let module_tree = kernel_parent.join(format!("modules-{}", self.kernel.version));
        if !module_tree.is_dir() {
            bail!("kernel module tree is missing: {}", module_tree.display());
        }
        let observed_module_tree_sha256 = sha256_regular_file_tree(&module_tree)?;
        if observed_module_tree_sha256 != self.module_tree_sha256 {
            bail!(
                "kernel module-tree digest mismatch for {}: expected {}, observed {}",
                module_tree.display(),
                self.module_tree_sha256,
                observed_module_tree_sha256
            );
        }
        Ok(())
    }
}

fn sha256_regular_file_tree(root: &Path) -> Result<String> {
    let mut relative_files = Vec::new();
    collect_regular_files(root, root, &mut relative_files)?;
    relative_files.sort_by(|left, right| {
        left.as_os_str()
            .as_bytes()
            .cmp(right.as_os_str().as_bytes())
    });

    let mut digest = Sha256::new();
    for relative in relative_files {
        let file_sha256 = sha256_file(&root.join(&relative))?;
        digest.update(file_sha256.as_bytes());
        digest.update(b"  ./");
        digest.update(relative.as_os_str().as_bytes());
        digest.update([0_u8]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn collect_regular_files(root: &Path, directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory)
        .with_context(|| format!("read module-tree directory {}", directory.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_dir() {
            collect_regular_files(root, &path, files)?;
        } else if file_type.is_file() {
            files.push(
                path.strip_prefix(root)
                    .with_context(|| {
                        format!(
                            "derive module-tree relative path for {} beneath {}",
                            path.display(),
                            root.display()
                        )
                    })?
                    .to_path_buf(),
            );
        }
    }
    Ok(())
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
            module_tree_digest_algorithm: MODULE_TREE_DIGEST_ALGORITHM.to_owned(),
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

    #[test]
    fn module_tree_digest_is_relocation_invariant_and_content_bound() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        fs::create_dir_all(first.path().join("nested")).unwrap();
        fs::create_dir_all(second.path().join("nested")).unwrap();
        fs::write(first.path().join("module.ko"), b"alpha\n").unwrap();
        fs::write(first.path().join("nested/modules.dep"), b"beta\n").unwrap();
        fs::write(second.path().join("module.ko"), b"alpha\n").unwrap();
        fs::write(second.path().join("nested/modules.dep"), b"beta\n").unwrap();

        let first_digest = sha256_regular_file_tree(first.path()).unwrap();
        let second_digest = sha256_regular_file_tree(second.path()).unwrap();
        assert_eq!(first_digest, second_digest);
        assert_eq!(
            first_digest,
            "4e32e48de7f3d6a8e51aac0d195754fcda675636c342cb152fa0d43d6a31139d"
        );

        fs::write(second.path().join("module.ko"), b"gamma\n").unwrap();
        assert_ne!(
            first_digest,
            sha256_regular_file_tree(second.path()).unwrap()
        );
    }
}
