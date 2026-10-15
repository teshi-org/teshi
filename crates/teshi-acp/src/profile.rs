//! User-level ACP agent definitions shared by every Teshi frontend.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use fd_lock::RwLock;
use serde::{Deserialize, Serialize};

use crate::{AcpAgentCommand, resolve_executable};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcpAgentProfile {
    pub id: String,
    pub name: String,
    pub program: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env_from: HashMap<String, String>,
}

impl AcpAgentProfile {
    pub fn command(&self, cwd: PathBuf) -> Result<AcpAgentCommand> {
        let mut env = self.env.clone();
        for (child_name, source_name) in &self.env_from {
            let value = std::env::var(source_name).with_context(|| {
                format!(
                    "ACP Agent {} requires environment variable {source_name} for {child_name}",
                    self.id
                )
            })?;
            env.insert(child_name.clone(), value);
        }
        Ok(AcpAgentCommand {
            program: self.program.clone(),
            args: self.args.clone(),
            cwd,
            env,
        })
    }
}

pub fn store_dir() -> Result<PathBuf> {
    Ok(dirs::data_local_dir()
        .context("user data directory is unavailable")?
        .join("teshi")
        .join("acp"))
}

pub fn config_path(root: &Path) -> PathBuf {
    root.join("installed.json")
}

pub fn load(root: &Path) -> Result<Vec<AcpAgentProfile>> {
    let path = config_path(root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let profiles: Vec<AcpAgentProfile> = serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("read {}", path.display()))?,
    )
    .with_context(|| format!("parse {}", path.display()))?;
    validate_profiles(&profiles).with_context(|| format!("validate {}", path.display()))?;
    Ok(profiles)
}

pub fn resolve_program(program: &Path) -> Result<PathBuf> {
    let text = program.to_string_lossy();
    let executable = if program.components().count() > 1 || program.is_absolute() {
        resolve_executable(Some(program), &text, None)?
    } else {
        resolve_executable(None, &text, std::env::var_os("PATH").as_deref())?
    };
    Ok(if executable.is_absolute() {
        executable
    } else {
        std::env::current_dir()?.join(executable)
    })
}

pub fn upsert(root: &Path, profile: AcpAgentProfile) -> Result<AcpAgentProfile> {
    validate_profile(&profile)?;
    let mut profiles = load(root)?;
    profiles.retain(|item| item.id != profile.id);
    profiles.push(profile.clone());
    sort_profiles(&mut profiles);
    save(root, &profiles)?;
    Ok(profile)
}

pub fn remove(root: &Path, id: &str) -> Result<AcpAgentProfile> {
    validate_id(id)?;
    let mut profiles = load(root)?;
    let index = profiles
        .iter()
        .position(|profile| profile.id == id)
        .with_context(|| format!("ACP Agent {id} is not configured"))?;
    let removed = profiles.remove(index);
    save(root, &profiles)?;
    Ok(removed)
}

pub fn validate_profiles(profiles: &[AcpAgentProfile]) -> Result<()> {
    let mut ids = HashSet::new();
    for profile in profiles {
        validate_profile(profile)?;
        if !ids.insert(&profile.id) {
            bail!("duplicate ACP Agent ID: {}", profile.id);
        }
    }
    Ok(())
}

pub fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("invalid ACP Agent ID: {id}");
    }
    Ok(())
}

fn validate_profile(profile: &AcpAgentProfile) -> Result<()> {
    validate_id(&profile.id)?;
    if profile.name.trim().is_empty() {
        bail!("ACP Agent {} has an empty name", profile.id);
    }
    if profile.program.as_os_str().is_empty() {
        bail!("ACP Agent {} has an empty program", profile.id);
    }
    for (child, source) in &profile.env_from {
        if child.is_empty() || source.is_empty() {
            bail!(
                "ACP Agent {} has an empty environment reference",
                profile.id
            );
        }
    }
    Ok(())
}

fn sort_profiles(profiles: &mut [AcpAgentProfile]) {
    profiles.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
}

fn save(root: &Path, profiles: &[AcpAgentProfile]) -> Result<()> {
    validate_profiles(profiles)?;
    fs::create_dir_all(root).with_context(|| format!("create {}", root.display()))?;
    let path = config_path(root);
    let lock_path = path.with_extension("lock");
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("open {}", lock_path.display()))?;
    let mut lock = RwLock::new(lock_file);
    let _guard = lock.write().context("lock ACP configuration")?;
    let temp = path.with_extension("tmp");
    let data = serde_json::to_vec_pretty(profiles).context("serialize ACP configuration")?;
    fs::write(&temp, data).with_context(|| format!("write {}", temp.display()))?;
    replace_file(&temp, &path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let ok = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str) -> AcpAgentProfile {
        AcpAgentProfile {
            id: id.into(),
            name: id.into(),
            program: PathBuf::from("agent"),
            args: vec!["acp".into()],
            env: HashMap::new(),
            env_from: HashMap::new(),
        }
    }

    #[test]
    fn loads_legacy_records_without_env_from() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            config_path(root.path()),
            r#"[{"id":"legacy","name":"Legacy","program":"agent","args":["acp"],"env":{"MODE":"test"}}]"#,
        )
        .unwrap();
        let loaded = load(root.path()).unwrap();
        assert!(loaded[0].env_from.is_empty());
        assert_eq!(loaded[0].env["MODE"], "test");
    }

    #[test]
    fn upsert_and_remove_are_deterministic() {
        let root = tempfile::tempdir().unwrap();
        upsert(root.path(), profile("second")).unwrap();
        upsert(root.path(), profile("first")).unwrap();
        let mut replacement = profile("first");
        replacement.name = "updated".into();
        upsert(root.path(), replacement.clone()).unwrap();
        assert_eq!(
            load(root.path()).unwrap(),
            vec![profile("second"), replacement]
        );
        assert_eq!(remove(root.path(), "second").unwrap().id, "second");
        assert_eq!(load(root.path()).unwrap().len(), 1);
    }

    #[test]
    fn invalid_file_is_not_rewritten_by_mutation() {
        let root = tempfile::tempdir().unwrap();
        let path = config_path(root.path());
        let original = br#"[{"id":"bad/id","name":"Bad","program":"agent"}]"#;
        fs::write(&path, original).unwrap();
        assert!(upsert(root.path(), profile("valid")).is_err());
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn command_resolves_environment_references_without_serializing_values() {
        let name = format!("TESHI_ACP_PROFILE_TEST_{}", std::process::id());
        unsafe { std::env::set_var(&name, "secret-value") };
        let mut value = profile("custom");
        value.env_from.insert("API_KEY".into(), name.clone());
        let command = value.command(PathBuf::from(".")).unwrap();
        assert_eq!(command.env["API_KEY"], "secret-value");
        assert!(
            !serde_json::to_string(&value)
                .unwrap()
                .contains("secret-value")
        );
        unsafe { std::env::remove_var(name) };
    }

    #[test]
    fn missing_environment_reference_names_variables_without_a_value() {
        let name = format!("TESHI_ACP_PROFILE_MISSING_{}", std::process::id());
        unsafe { std::env::remove_var(&name) };
        let mut value = profile("custom");
        value.env_from.insert("API_KEY".into(), name.clone());
        let error = value.command(PathBuf::from(".")).unwrap_err().to_string();
        assert!(error.contains("custom"));
        assert!(error.contains("API_KEY"));
        assert!(error.contains(&name));
    }
}
