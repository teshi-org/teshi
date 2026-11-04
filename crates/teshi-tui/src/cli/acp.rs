use std::{collections::HashMap, io::Write, path::Path};

use anyhow::{Context, Result, bail};

use super::AcpCommand;

pub fn handle_acp_command(action: &AcpCommand) -> Result<()> {
    let root = teshi_acp::profile::store_dir()?;
    handle_acp_command_at(action, &root, &mut std::io::stdout())
}

fn handle_acp_command_at(action: &AcpCommand, root: &Path, writer: &mut dyn Write) -> Result<()> {
    use teshi_acp::profile;

    match action {
        AcpCommand::ConfigPath => {
            writeln!(writer, "{}", profile::config_path(root).display())?;
        }
        AcpCommand::List { json } => {
            let profiles = profile::load(root)?;
            if *json {
                writeln!(writer, "{}", serde_json::to_string_pretty(&profiles)?)?;
            } else if profiles.is_empty() {
                writeln!(writer, "No ACP agents configured.")?;
            } else {
                for item in profiles {
                    writeln!(
                        writer,
                        "{}\t{}\t{}",
                        item.id,
                        item.name,
                        item.program.display()
                    )?;
                }
            }
        }
        AcpCommand::Show { id, json } => {
            let item = find(root, id)?;
            if *json {
                writeln!(writer, "{}", serde_json::to_string_pretty(&item)?)?;
            } else {
                writeln!(writer, "ID: {}", item.id)?;
                writeln!(writer, "Name: {}", item.name)?;
                writeln!(writer, "Program: {}", item.program.display())?;
                writeln!(writer, "Arguments: {}", serde_json::to_string(&item.args)?)?;
                if !item.env_from.is_empty() {
                    let mut references: Vec<_> = item.env_from.iter().collect();
                    references.sort();
                    for (child, source) in references {
                        writeln!(writer, "Environment: {child} <- {source}")?;
                    }
                }
            }
        }
        AcpCommand::Add {
            id,
            program,
            name,
            args,
            env_from,
            json,
        } => {
            let resolved = profile::resolve_program(program)?;
            let item = profile::AcpAgentProfile {
                id: id.clone(),
                name: name.clone().unwrap_or_else(|| id.clone()),
                program: resolved,
                args: args.clone(),
                env: HashMap::new(),
                env_from: parse_env_references(env_from)?,
            };
            let item = profile::upsert(root, item)?;
            if *json {
                writeln!(writer, "{}", serde_json::to_string_pretty(&item)?)?;
            } else {
                writeln!(writer, "Configured ACP Agent {} ({}).", item.name, item.id)?;
            }
        }
        AcpCommand::Remove { id, json } => {
            let item = profile::remove(root, id)?;
            if *json {
                writeln!(writer, "{}", serde_json::to_string_pretty(&item)?)?;
            } else {
                writeln!(writer, "Removed ACP Agent {} ({}).", item.name, item.id)?;
            }
        }
    }
    Ok(())
}

fn find(root: &Path, id: &str) -> Result<teshi_acp::profile::AcpAgentProfile> {
    teshi_acp::profile::load(root)?
        .into_iter()
        .find(|item| item.id == id)
        .with_context(|| format!("ACP Agent {id} is not configured"))
}

fn parse_env_references(values: &[String]) -> Result<HashMap<String, String>> {
    let mut references = HashMap::new();
    for value in values {
        let (child, source) = value
            .split_once('=')
            .map_or((value.as_str(), value.as_str()), |(child, source)| {
                (child, source)
            });
        if child.is_empty() || source.is_empty() || child.contains('=') || source.contains('=') {
            bail!("invalid environment reference {value:?}; expected NAME or NAME=SOURCE_ENV");
        }
        if references
            .insert(child.to_string(), source.to_string())
            .is_some()
        {
            bail!("duplicate child environment variable: {child}");
        }
    }
    Ok(references)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use clap::Parser;

    #[test]
    fn parses_structured_add_command() {
        let cli = Cli::try_parse_from([
            "teshi",
            "acp",
            "add",
            "local",
            "--program",
            "agent",
            "--name",
            "Local Agent",
            "--arg",
            "acp",
            "--arg=--stdio",
            "--env",
            "API_KEY=MY_AGENT_KEY",
        ])
        .unwrap();
        let Some(crate::cli::Command::Acp {
            action: AcpCommand::Add {
                id, args, env_from, ..
            },
        }) = cli.command
        else {
            panic!("expected ACP add command");
        };
        assert_eq!(id, "local");
        assert_eq!(args, ["acp", "--stdio"]);
        assert_eq!(env_from, ["API_KEY=MY_AGENT_KEY"]);
    }

    #[test]
    fn json_output_never_resolves_environment_values() {
        let root = tempfile::tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let action = AcpCommand::Add {
            id: "local".into(),
            program: executable,
            name: Some("Local Agent".into()),
            args: vec!["acp".into()],
            env_from: vec!["API_KEY=VERY_SECRET_HOST_NAME".into()],
            json: true,
        };
        let mut output = Vec::new();
        handle_acp_command_at(&action, root.path(), &mut output).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("VERY_SECRET_HOST_NAME"));
        assert!(!text.contains("secret-value"));
    }

    #[test]
    fn failed_add_preserves_existing_configuration() {
        let root = tempfile::tempdir().unwrap();
        let path = teshi_acp::profile::config_path(root.path());
        std::fs::write(&path, b"[]").unwrap();
        let before = std::fs::read(&path).unwrap();
        let action = AcpCommand::Add {
            id: "local".into(),
            program: root.path().join("missing-agent"),
            name: None,
            args: vec![],
            env_from: vec![],
            json: false,
        };
        assert!(handle_acp_command_at(&action, root.path(), &mut Vec::new()).is_err());
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
}
