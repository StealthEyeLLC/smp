use crate::error::{Result, SmpError};
use crate::util::{ensure_beneath, reject_symlink_components, validate_absolute_clean};
use serde::Serialize;
use std::env;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Paths {
    pub binary: PathBuf,
    pub lib: PathBuf,
    pub config: PathBuf,
    pub credentials: PathBuf,
    pub state: PathBuf,
    pub assets: PathBuf,
    pub machines: PathBuf,
    pub requests: PathBuf,
    pub results: PathBuf,
    pub provenance: PathBuf,
    pub runtime: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryContract {
    pub path: String,
    pub purpose: String,
    pub mode: String,
    pub persistent: bool,
    pub cleanup: String,
}

impl Paths {
    pub fn system() -> Result<Self> {
        let binary = env_path("SMP_BINARY", "/usr/local/bin/smp")?;
        let lib = env_path("SMP_LIB_ROOT", "/usr/lib/smp")?;
        let config = env_path("SMP_CONFIG_ROOT", "/etc/smp")?;
        let state = env_path("SMP_STATE_ROOT", "/var/lib/smp")?;
        let runtime = env_path("SMP_RUNTIME_ROOT", "/run/smp")?;
        let paths = Self {
            binary,
            credentials: config.join("credentials"),
            assets: state.join("assets"),
            machines: state.join("machines"),
            requests: state.join("requests"),
            results: state.join("results"),
            provenance: state.join("provenance"),
            lib,
            config,
            state,
            runtime,
        };
        paths.validate()?;
        Ok(paths)
    }

    pub fn rooted(root: &Path) -> Result<Self> {
        validate_absolute_clean(root)?;
        let config = root.join("etc/smp");
        let state = root.join("var/lib/smp");
        let paths = Self {
            binary: root.join("usr/local/bin/smp"),
            lib: root.join("usr/lib/smp"),
            credentials: config.join("credentials"),
            assets: state.join("assets"),
            machines: state.join("machines"),
            requests: state.join("requests"),
            results: state.join("results"),
            provenance: state.join("provenance"),
            runtime: root.join("run/smp"),
            config,
            state,
        };
        paths.validate()?;
        Ok(paths)
    }

    pub fn validate(&self) -> Result<()> {
        for path in [
            &self.binary,
            &self.lib,
            &self.config,
            &self.credentials,
            &self.state,
            &self.assets,
            &self.machines,
            &self.requests,
            &self.results,
            &self.provenance,
            &self.runtime,
        ] {
            validate_absolute_clean(path)?;
        }
        for path in [
            &self.assets,
            &self.machines,
            &self.requests,
            &self.results,
            &self.provenance,
        ] {
            ensure_beneath(&self.state, path)?;
        }
        ensure_beneath(&self.config, &self.credentials)?;
        Ok(())
    }

    pub fn ensure_layout(&self) -> Result<()> {
        self.validate()?;
        for (path, mode) in [
            (&self.lib, 0o755),
            (&self.config, 0o755),
            (&self.credentials, 0o700),
            (&self.state, 0o700),
            (&self.assets, 0o755),
            (&self.machines, 0o700),
            (&self.requests, 0o700),
            (&self.results, 0o700),
            (&self.provenance, 0o700),
            (&self.runtime, 0o755),
        ] {
            reject_symlink_components(path)?;
            fs::create_dir_all(path)
                .map_err(|error| SmpError::io(path.display().to_string(), error))?;
            fs::set_permissions(path, fs::Permissions::from_mode(mode))
                .map_err(|error| SmpError::io(path.display().to_string(), error))?;
        }
        Ok(())
    }

    pub fn machine_dir(&self, machine: &str) -> Result<PathBuf> {
        validate_machine_name(machine)?;
        let path = self.machines.join(machine);
        ensure_beneath(&self.machines, &path)?;
        Ok(path)
    }

    pub fn machine_record(&self, machine: &str) -> Result<PathBuf> {
        Ok(self.machine_dir(machine)?.join("machine.json"))
    }

    pub fn machine_socket(&self, machine: &str) -> Result<PathBuf> {
        validate_machine_name(machine)?;
        let path = self.runtime.join(format!("{machine}.sock"));
        ensure_beneath(&self.runtime, &path)?;
        if path.as_os_str().as_bytes().len() > 107 {
            return Err(SmpError::Invalid(format!(
                "Firecracker API socket path exceeds the Unix limit: {}",
                path.display()
            )));
        }
        Ok(path)
    }

    pub fn request_record(&self, request_id: &str) -> Result<PathBuf> {
        validate_record_id(request_id)?;
        Ok(self.requests.join(format!("{request_id}.json")))
    }

    pub fn result_dir(&self, request_id: &str) -> Result<PathBuf> {
        validate_record_id(request_id)?;
        Ok(self.results.join(request_id))
    }

    pub fn directory_contract(&self) -> Vec<DirectoryContract> {
        vec![
            contract(
                &self.lib,
                "installed SMP support files",
                "0755",
                true,
                "removed with binary-and-service removal",
            ),
            contract(
                &self.config,
                "non-secret SMP configuration",
                "0755",
                true,
                "preserved unless complete removal is explicit",
            ),
            contract(
                &self.credentials,
                "SMP-only credentials",
                "0700",
                true,
                "removed only by explicit credential or complete removal",
            ),
            contract(
                &self.state,
                "authoritative SMP state",
                "0700",
                true,
                "preserved by ordinary uninstall",
            ),
            contract(
                &self.assets,
                "verified canonical assets",
                "0755",
                true,
                "removed only by explicit state cleanup",
            ),
            contract(
                &self.machines,
                "machine definitions and writable disks",
                "0700",
                true,
                "persistent disks require explicit deletion",
            ),
            contract(
                &self.requests,
                "minimal request replay records",
                "0700",
                true,
                "terminal records expire by advertised retention",
            ),
            contract(
                &self.results,
                "bounded detached and overflow output",
                "0700",
                true,
                "terminal results expire by advertised retention",
            ),
            contract(
                &self.provenance,
                "installed source and asset identities",
                "0700",
                true,
                "archived before replacement",
            ),
            contract(
                &self.runtime,
                "sockets locks and transient process state",
                "0755",
                false,
                "recreated at service start",
            ),
        ]
    }
}

fn contract(
    path: &Path,
    purpose: &str,
    mode: &str,
    persistent: bool,
    cleanup: &str,
) -> DirectoryContract {
    DirectoryContract {
        path: path.display().to_string(),
        purpose: purpose.to_owned(),
        mode: mode.to_owned(),
        persistent,
        cleanup: cleanup.to_owned(),
    }
}

fn env_path(variable: &str, default: &str) -> Result<PathBuf> {
    let path = PathBuf::from(env::var_os(variable).unwrap_or_else(|| default.into()));
    validate_absolute_clean(&path)?;
    Ok(path)
}

pub fn validate_machine_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 63 {
        return Err(SmpError::Invalid(
            "machine name must contain 1 through 63 bytes".to_owned(),
        ));
    }
    let bytes = name.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        || !bytes.last().is_some_and(u8::is_ascii_alphanumeric)
    {
        return Err(SmpError::Invalid(
            "machine name must start and end with an ASCII letter or digit".to_owned(),
        ));
    }
    if !bytes
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(SmpError::Invalid(format!("invalid machine name: {name}")));
    }
    Ok(())
}

pub fn validate_record_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
        || value == "."
        || value == ".."
    {
        return Err(SmpError::Invalid(format!("invalid record ID: {value}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_name_validation_is_canonical() {
        for valid in ["default", "build-01", "a_b", "Z9"] {
            assert!(validate_machine_name(valid).is_ok(), "{valid}");
        }
        for invalid in ["", "../x", "a/b", "-bad", "bad-", "a.b", ".."] {
            assert!(validate_machine_name(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn machine_socket_uses_runtime_and_enforces_unix_limit() -> Result<()> {
        let paths = Paths::rooted(Path::new("/isolation"))?;
        assert_eq!(
            paths.machine_socket("default")?,
            PathBuf::from("/isolation/run/smp/default.sock")
        );
        let long_root = PathBuf::from(format!("/{}", "x".repeat(100)));
        assert!(
            Paths::rooted(&long_root)?
                .machine_socket("default")
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn request_ids_cannot_traverse() {
        assert!(validate_record_id("018f6f3d-3e1a").is_ok());
        assert!(validate_record_id("../../etc").is_err());
        assert!(validate_record_id("a/b").is_err());
    }
}
