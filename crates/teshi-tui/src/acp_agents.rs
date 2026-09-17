//! User-level ACP installation records. Selection is deliberately not persisted.
use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use teshi_acp::registry::{RegistryAgent, ResolvedDistribution};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledAgent {
    pub id: String,
    pub name: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

impl InstalledAgent {
    pub fn command(&self, cwd: PathBuf) -> teshi_acp::AcpAgentCommand {
        teshi_acp::AcpAgentCommand {
            program: self.program.clone(),
            args: self.args.clone(),
            cwd,
            env: self.env.clone(),
        }
    }
}

pub fn store_dir() -> Result<PathBuf> {
    Ok(dirs::data_local_dir()
        .context("user data directory is unavailable")?
        .join("teshi")
        .join("acp"))
}

fn manifest(root: &Path) -> PathBuf {
    root.join("installed.json")
}

pub fn load(root: &Path) -> Result<Vec<InstalledAgent>> {
    let path = manifest(root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    serde_json::from_slice(&fs::read(&path).with_context(|| format!("read {}", path.display()))?)
        .with_context(|| format!("parse {}", path.display()))
}

fn save(root: &Path, agents: &[InstalledAgent]) -> Result<()> {
    fs::create_dir_all(root)?;
    teshi_engine::write_atomic(&manifest(root), &agents)
}

pub fn install(root: &Path, agent: &RegistryAgent) -> Result<InstalledAgent> {
    validate_id(&agent.id)?;
    let platform = teshi_acp::registry::current_platform()?;
    let distribution = agent.resolve_for_platform(&platform)?;
    let target = root.join("packages").join(&agent.id);
    let installed = match distribution {
        ResolvedDistribution::Binary(binary) => install_binary(&target, binary, agent)?,
        ResolvedDistribution::Npx { package, args } => {
            fs::create_dir_all(&target)?;
            let output = Command::new(if cfg!(windows) { "npm.cmd" } else { "npm" })
                .args([
                    "install",
                    "--prefix",
                    &target.to_string_lossy(),
                    "--",
                    package,
                ])
                .output()
                .context("launch npm to install Registry package")?;
            if !output.status.success() {
                bail!("npm install failed: {}", output.status);
            }
            let package_name = package
                .rfind('@')
                .filter(|index| *index > 0)
                .map_or(package, |index| &package[..index]);
            let metadata: serde_json::Value = serde_json::from_slice(
                &fs::read(
                    target
                        .join("node_modules")
                        .join(package_name)
                        .join("package.json"),
                )
                .context("installed npm package metadata missing")?,
            )?;
            let binary_name = match &metadata["bin"] {
                serde_json::Value::String(_) => package_name.rsplit('/').next(),
                serde_json::Value::Object(bins) if bins.len() == 1 => {
                    bins.keys().next().map(String::as_str)
                }
                serde_json::Value::Object(bins) => bins
                    .keys()
                    .find(|name| *name == package_name.rsplit('/').next().unwrap_or(""))
                    .map(String::as_str),
                _ => None,
            }
            .context("Registry npm package has no unambiguous executable")?;
            let executable = target
                .join("node_modules")
                .join(".bin")
                .join(if cfg!(windows) {
                    format!("{binary_name}.cmd")
                } else {
                    binary_name.into()
                });
            if !executable.is_file() {
                bail!("installed npm executable is missing");
            }
            InstalledAgent {
                id: agent.id.clone(),
                name: agent.name.clone().unwrap_or_else(|| agent.id.clone()),
                program: executable,
                args: args.to_vec(),
                env: package_env(agent.distribution.npx.as_ref()),
            }
        }
        ResolvedDistribution::Uvx { package, args } => {
            let bin = target.join("bin");
            fs::create_dir_all(&bin)?;
            let mut env = HashMap::new();
            env.insert(
                "UV_TOOL_DIR".into(),
                target.join("tools").to_string_lossy().into_owned(),
            );
            env.insert("UV_TOOL_BIN_DIR".into(), bin.to_string_lossy().into_owned());
            let output = Command::new("uv")
                .args(["tool", "install", package])
                .envs(&env)
                .output()
                .context("launch uv to install Registry package")?;
            if !output.status.success() {
                bail!("uv tool install failed: {}", output.status);
            }
            let name = package.split(['@', '=']).next().unwrap_or(package);
            let executable = bin.join(if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.into()
            });
            if !executable.is_file() {
                bail!("uv installed package but executable {name} was not found");
            }
            InstalledAgent {
                id: agent.id.clone(),
                name: agent.name.clone().unwrap_or_else(|| agent.id.clone()),
                program: executable,
                args: args.to_vec(),
                env: {
                    env.extend(package_env(agent.distribution.uvx.as_ref()));
                    env
                },
            }
        }
    };
    record(root, installed)
}

fn package_env(
    package: Option<&teshi_acp::registry::PackageDistribution>,
) -> HashMap<String, String> {
    package
        .and_then(|package| package.extra.get("env"))
        .and_then(serde_json::Value::as_object)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("invalid ACP Agent ID");
    }
    Ok(())
}

fn record(root: &Path, installed: InstalledAgent) -> Result<InstalledAgent> {
    let mut agents = load(root)?;
    agents.retain(|item| item.id != installed.id);
    agents.push(installed.clone());
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    save(root, &agents)?;
    Ok(installed)
}

pub fn add_custom(root: &Path, id: &str, argv: &[String]) -> Result<InstalledAgent> {
    validate_id(id)?;
    let (program, args) = argv.split_first().context("custom ACP command is empty")?;
    let path = Path::new(program);
    let executable = if path.components().count() > 1 || path.is_absolute() {
        teshi_acp::resolve_executable(Some(path), program, None)?
    } else {
        teshi_acp::resolve_executable(None, program, std::env::var_os("PATH").as_deref())?
    };
    let executable = if executable.is_absolute() {
        executable
    } else {
        std::env::current_dir()?.join(executable)
    };
    record(
        root,
        InstalledAgent {
            id: id.into(),
            name: id.into(),
            program: executable,
            args: args.to_vec(),
            env: HashMap::new(),
        },
    )
}

fn install_binary(
    target: &Path,
    binary: &teshi_acp::registry::BinaryTarget,
    agent: &RegistryAgent,
) -> Result<InstalledAgent> {
    let command = Path::new(&binary.cmd);
    if command.is_absolute()
        || command.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        })
    {
        bail!("unsafe Registry command path");
    }
    let url = reqwest::Url::parse(&binary.archive).context("invalid Registry archive URL")?;
    if url.scheme() != "https" {
        bail!("Registry archive must use HTTPS");
    }
    let bytes = tokio::runtime::Runtime::new()?
        .block_on(async {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()?
                .get(url)
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await
        })
        .context("download Registry archive")?;
    install_binary_payload(target, binary, agent, &bytes)
}

fn install_binary_payload(
    target: &Path,
    binary: &teshi_acp::registry::BinaryTarget,
    agent: &RegistryAgent,
    bytes: &[u8],
) -> Result<InstalledAgent> {
    let command = Path::new(&binary.cmd);
    if let Some(expected) = &binary.sha256 {
        let actual = hex::encode(Sha256::digest(bytes));
        if !actual.eq_ignore_ascii_case(expected) {
            bail!("Registry archive SHA-256 mismatch");
        }
    }
    let staging = target.with_extension("installing");
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;
    let archive_result: Result<()> = if binary.archive.ends_with(".zip") {
        let mut archive = zip::ZipArchive::new(io::Cursor::new(&bytes))?;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let relative = entry.enclosed_name().context("unsafe archive path")?;
            let destination = staging.join(relative);
            if entry.is_dir() {
                fs::create_dir_all(&destination)?;
            } else {
                fs::create_dir_all(destination.parent().unwrap())?;
                io::copy(&mut entry, &mut fs::File::create(destination)?)?;
            }
        }
        Ok(())
    } else if binary.archive.ends_with(".tar.gz") || binary.archive.ends_with(".tgz") {
        let decoder = flate2::read::GzDecoder::new(io::Cursor::new(&bytes));
        let mut archive = tar::Archive::new(decoder);
        for entry in archive.entries()? {
            let mut entry = entry?;
            if entry.header().entry_type().is_file() || entry.header().entry_type().is_dir() {
                entry.unpack_in(&staging)?;
            } else {
                bail!("unsupported archive entry type");
            }
        }
        Ok(())
    } else {
        bail!("unsupported Registry archive format");
    };
    archive_result?;
    let executable = staging.join(command);
    if !executable.is_file() {
        bail!("Registry executable missing from archive: {}", binary.cmd);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&executable)?.permissions();
        permissions.set_mode(permissions.mode() | 0o700);
        fs::set_permissions(&executable, permissions)?;
    }
    if target.exists() {
        fs::remove_dir_all(target)?;
    }
    fs::rename(&staging, target)?;
    Ok(InstalledAgent {
        id: agent.id.clone(),
        name: agent.name.clone().unwrap_or_else(|| agent.id.clone()),
        program: target.join(command),
        args: binary.args.clone(),
        env: HashMap::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifest_round_trip_does_not_select_agent() {
        let root = tempfile::tempdir().unwrap();
        let agent = InstalledAgent {
            id: "sample".into(),
            name: "Sample".into(),
            program: "sample".into(),
            args: vec!["acp".into()],
            env: HashMap::new(),
        };
        save(root.path(), std::slice::from_ref(&agent)).unwrap();
        assert_eq!(load(root.path()).unwrap(), vec![agent]);
    }
    #[test]
    fn rejects_registry_path_ids() {
        let root = tempfile::tempdir().unwrap();
        let registry = teshi_acp::registry::Registry::parse(
            r#"{"agents":[{"id":"../escape","distribution":{"npx":{"package":"x"}}}]}"#,
        )
        .unwrap();
        assert!(install(root.path(), &registry.agents[0]).is_err());
        assert!(load(root.path()).unwrap().is_empty());
    }

    #[test]
    fn failed_binary_install_never_becomes_installed() {
        let root = tempfile::tempdir().unwrap();
        let platform = teshi_acp::registry::current_platform().unwrap();
        let json = format!(
            r#"{{"agents":[{{"id":"bad","distribution":{{"binary":{{"{platform}":{{"archive":"https://example.invalid/agent.zip","cmd":"../escape"}}}}}}}}]}}"#
        );
        let registry = teshi_acp::registry::Registry::parse(&json).unwrap();
        assert!(
            install(root.path(), &registry.agents[0])
                .unwrap_err()
                .to_string()
                .contains("unsafe Registry command")
        );
        assert!(load(root.path()).unwrap().is_empty());
    }

    #[test]
    fn explicit_custom_command_is_recorded() {
        let root = tempfile::tempdir().unwrap();
        let executable = root
            .path()
            .join(if cfg!(windows) { "agent.exe" } else { "agent" });
        fs::write(&executable, b"").unwrap();
        let argv = vec![executable.to_string_lossy().into_owned(), "acp".into()];
        let installed = add_custom(root.path(), "custom", &argv).unwrap();
        assert_eq!(installed.program, executable);
        assert_eq!(installed.args, ["acp"]);
        assert_eq!(load(root.path()).unwrap(), vec![installed]);
    }

    #[test]
    fn verified_registry_zip_produces_launchable_record() {
        use std::io::Write;
        let root = tempfile::tempdir().unwrap();
        let mut archive = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        archive
            .start_file("agent.exe", zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(b"fixture executable").unwrap();
        let bytes = archive.finish().unwrap().into_inner();
        let digest = hex::encode(Sha256::digest(&bytes));
        let registry = teshi_acp::registry::Registry::parse(&format!(
            r#"{{"agents":[{{"id":"fixture","name":"Fixture","distribution":{{"binary":{{"windows-x86_64":{{"archive":"https://example.invalid/fixture.zip","cmd":"agent.exe","args":["acp"],"sha256":"{digest}"}}}}}}}}]}}"#
        )).unwrap();
        let binary = registry.agents[0]
            .distribution
            .binary
            .as_ref()
            .unwrap()
            .get("windows-x86_64")
            .unwrap();
        let target = root.path().join("packages").join("fixture");
        let installed =
            install_binary_payload(&target, binary, &registry.agents[0], &bytes).unwrap();
        assert_eq!(installed.program, target.join("agent.exe"));
        assert_eq!(installed.args, ["acp"]);
        assert_eq!(fs::read(&installed.program).unwrap(), b"fixture executable");
        let mut corrupted = bytes;
        corrupted[0] ^= 1;
        assert!(
            install_binary_payload(&target, binary, &registry.agents[0], &corrupted)
                .unwrap_err()
                .to_string()
                .contains("SHA-256 mismatch")
        );
        assert_eq!(fs::read(&installed.program).unwrap(), b"fixture executable");
    }
}
