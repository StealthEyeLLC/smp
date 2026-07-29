use crate::error::{Result, SmpError};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).map_err(|error| SmpError::io(path.display().to_string(), error))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 1024 * 128];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| SmpError::io(path.display().to_string(), error))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex::encode(digest.finalize()))
}

pub fn normalize_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut normalized = serde_json::Map::new();
            for key in keys {
                if let Some(value) = object.get(key) {
                    normalized.insert(key.clone(), normalize_json(value));
                }
            }
            Value::Object(normalized)
        }
        Value::Array(values) => Value::Array(values.iter().map(normalize_json).collect()),
        _ => value.clone(),
    }
}

pub fn canonical_json_digest<T: Serialize>(value: &T) -> Result<String> {
    let value =
        serde_json::to_value(value).map_err(|error| SmpError::json("<canonical-json>", error))?;
    let bytes = serde_json::to_vec(&normalize_json(&value))
        .map_err(|error| SmpError::json("<canonical-json>", error))?;
    Ok(sha256_bytes(&bytes))
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).map_err(|error| SmpError::io(path.display().to_string(), error))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| SmpError::json(path.display().to_string(), error))
}

pub fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| SmpError::Invalid(format!("path has no parent: {}", path.display())))?;
    fs::create_dir_all(parent)
        .map_err(|error| SmpError::io(parent.display().to_string(), error))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("smp"),
        Uuid::new_v4()
    ));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(mode)
            .open(&temporary)
            .map_err(|error| SmpError::io(temporary.display().to_string(), error))?;
        file.write_all(bytes)
            .map_err(|error| SmpError::io(temporary.display().to_string(), error))?;
        file.sync_all()
            .map_err(|error| SmpError::io(temporary.display().to_string(), error))?;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))
            .map_err(|error| SmpError::io(temporary.display().to_string(), error))?;
        fs::rename(&temporary, path)
            .map_err(|error| SmpError::io(path.display().to_string(), error))?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| SmpError::io(parent.display().to_string(), error))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn atomic_json<T: Serialize>(path: &Path, value: &T, mode: u32) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| SmpError::json(path.display().to_string(), error))?;
    bytes.push(b'\n');
    atomic_write(path, &bytes, mode)
}

pub fn validate_absolute_clean(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(SmpError::Invalid(format!(
            "path must be absolute: {}",
            path.display()
        )));
    }
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err(SmpError::Invalid(format!(
                "path contains traversal: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

pub fn ensure_beneath(root: &Path, path: &Path) -> Result<()> {
    validate_absolute_clean(root)?;
    validate_absolute_clean(path)?;
    if path == root || path.starts_with(root) {
        Ok(())
    } else {
        Err(SmpError::Invalid(format!(
            "{} is outside {}",
            path.display(),
            root.display()
        )))
    }
}

pub fn reject_symlink_components(path: &Path) -> Result<()> {
    let mut cursor = PathBuf::new();
    for component in path.components() {
        cursor.push(component.as_os_str());
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(SmpError::Invalid(format!(
                    "symlink component rejected: {}",
                    cursor.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(SmpError::io(cursor.display().to_string(), error)),
        }
    }
    Ok(())
}

pub fn physical_size(path: &Path) -> Result<u64> {
    let metadata =
        fs::metadata(path).map_err(|error| SmpError::io(path.display().to_string(), error))?;
    Ok(metadata.blocks().saturating_mul(512))
}

pub fn command_output(program: &str, args: &[String]) -> Result<Output> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| SmpError::io(program, error))
}

pub fn command_checked(program: &str, args: &[String]) -> Result<Output> {
    let output = command_output(program, args)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(SmpError::External {
            program: program.to_owned(),
            code: output.status.code().unwrap_or(128),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

pub fn path_mode(path: &Path) -> Result<u32> {
    fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o7777)
        .map_err(|error| SmpError::io(path.display().to_string(), error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_digest_ignores_object_order() -> Result<()> {
        let left = json!({"a": 1, "b": {"x": 2, "y": 3}});
        let right = json!({"b": {"y": 3, "x": 2}, "a": 1});
        assert_eq!(
            canonical_json_digest(&left)?,
            canonical_json_digest(&right)?
        );
        Ok(())
    }

    #[test]
    fn atomic_write_replaces_and_sets_mode() -> Result<()> {
        let directory = tempfile::tempdir().map_err(|error| SmpError::io("tempdir", error))?;
        let path = directory.path().join("state.json");
        atomic_write(&path, b"one", 0o600)?;
        atomic_write(&path, b"two", 0o640)?;
        assert_eq!(
            fs::read(&path).map_err(|error| SmpError::io(path.display().to_string(), error))?,
            b"two"
        );
        assert_eq!(path_mode(&path)?, 0o640);
        Ok(())
    }

    #[test]
    fn path_confinement_rejects_escape() {
        assert!(ensure_beneath(Path::new("/var/lib/smp"), Path::new("/etc/passwd")).is_err());
        assert!(
            ensure_beneath(
                Path::new("/var/lib/smp"),
                Path::new("/var/lib/smp/machines/default")
            )
            .is_ok()
        );
    }
}
