use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
};

use tokio::process::Command;

use crate::error::{AcpError, AcpResult, redact_secret};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpAgentCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
}

impl AcpAgentCommand {
    pub fn to_tokio_command(&self) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args).current_dir(&self.cwd);
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        cmd
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorAcpConfig {
    pub executable: Option<PathBuf>,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
}

impl CursorAcpConfig {
    pub fn command_with_path(&self, path: PathBuf) -> AcpAgentCommand {
        AcpAgentCommand {
            program: path,
            args: vec!["acp".into()],
            cwd: self.cwd.clone(),
            env: self.env.clone(),
        }
    }

    pub fn command(&self) -> AcpResult<AcpAgentCommand> {
        let program = resolve_executable(
            self.executable.as_deref(),
            "agent",
            env::var_os("PATH").as_deref(),
        )?;
        Ok(self.command_with_path(program))
    }
}

pub fn resolve_executable(
    configured: Option<&Path>,
    default_name: &str,
    path_var: Option<&std::ffi::OsStr>,
) -> AcpResult<PathBuf> {
    if let Some(path) = configured {
        if path.is_file() {
            return Ok(path.to_path_buf());
        }
        return Err(AcpError::ConfiguredExecutableMissing {
            path: path.to_path_buf(),
        });
    }
    find_on_path(default_name, path_var).ok_or_else(|| AcpError::ExecutableNotFound {
        program: default_name.into(),
    })
}

fn find_on_path(name: &str, path_var: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let paths = env::split_paths(path_var?);
    #[cfg(windows)]
    let exts: Vec<String> = env::var_os("PATHEXT")
        .map(|v| {
            env::split_paths(&v)
                .map(|p| p.to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_else(|| vec![".exe".into(), ".cmd".into(), ".bat".into()]);
    #[cfg(not(windows))]
    let exts: Vec<String> = vec![String::new()];
    for dir in paths {
        if dir.as_os_str().is_empty() || dir == Path::new(".") {
            continue;
        }
        for ext in &exts {
            let candidate = if ext.is_empty()
                || name
                    .to_ascii_lowercase()
                    .ends_with(&ext.to_ascii_lowercase())
            {
                dir.join(name)
            } else {
                dir.join(format!("{name}{ext}"))
            };
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentRedaction(pub Vec<(String, String)>);

impl EnvironmentRedaction {
    pub fn from_env(env: &HashMap<String, String>) -> Self {
        Self(
            env.iter()
                .map(|(k, v)| (k.clone(), redact_secret(&format!("{k}={v}"))))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_command_is_structured() {
        let cwd = PathBuf::from("/project");
        let cfg = CursorAcpConfig {
            executable: None,
            cwd: cwd.clone(),
            env: HashMap::new(),
        };
        let cmd = cfg.command_with_path(PathBuf::from("agent"));
        assert_eq!(cmd.program, PathBuf::from("agent"));
        assert_eq!(cmd.args, vec!["acp"]);
        assert_eq!(cmd.cwd, cwd);
    }

    #[test]
    fn resolves_configured_and_path_without_cwd_search() {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp
            .path()
            .join(if cfg!(windows) { "agent.exe" } else { "agent" });
        std::fs::write(&exe, "").unwrap();
        assert_eq!(resolve_executable(Some(&exe), "agent", None).unwrap(), exe);
        assert!(resolve_executable(Some(&temp.path().join("missing")), "agent", None).is_err());
        assert!(resolve_executable(None, "agent", Some(temp.path().as_os_str())).is_ok());
        assert!(resolve_executable(None, "agent", None).is_err());
    }

    #[test]
    fn redacts_secret_env() {
        let mut env = HashMap::new();
        env.insert("CURSOR_API_KEY".into(), "super-secret".into());
        let dbg = format!("{:?}", EnvironmentRedaction::from_env(&env));
        assert!(!dbg.contains("super-secret"));
    }
}
