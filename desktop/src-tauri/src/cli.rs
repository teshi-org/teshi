use std::path::PathBuf;

use clap::Parser;

/// CLI options for the `teshi-desktop` binary.
#[derive(Debug, Parser)]
#[command(name = "teshi-desktop", version, about = "teshi native desktop shell")]
pub struct DesktopCli {
    /// Project directory to open on startup
    #[arg(long)]
    pub project: Option<PathBuf>,
    /// Project directory (shortcut for `--project`)
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,
}

impl DesktopCli {
    /// Returns the project path from `--project` or the positional argument.
    pub fn project_path(&self) -> Option<PathBuf> {
        self.project.clone().or_else(|| self.path.clone())
    }

    /// Parses a project path from process arguments (for single-instance re-launch).
    pub fn project_from_argv(argv: &[String]) -> Option<String> {
        let args = argv.get(1..)?;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--project" if i + 1 < args.len() => {
                    return Some(args[i + 1].clone());
                }
                s if !s.starts_with('-') => return Some(s.to_string()),
                _ => {}
            }
            i += 1;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_from_argv_reads_long_flag() {
        let argv = vec![
            "teshi-desktop".into(),
            "--project".into(),
            "/tmp/proj".into(),
        ];
        assert_eq!(
            DesktopCli::project_from_argv(&argv).as_deref(),
            Some("/tmp/proj")
        );
    }

    #[test]
    fn project_from_argv_reads_positional() {
        let argv = vec!["teshi-desktop".into(), "/tmp/proj".into()];
        assert_eq!(
            DesktopCli::project_from_argv(&argv).as_deref(),
            Some("/tmp/proj")
        );
    }

    #[test]
    fn project_path_prefers_long_flag() {
        let cli = DesktopCli {
            project: Some(PathBuf::from("/a")),
            path: Some(PathBuf::from("/b")),
        };
        assert_eq!(cli.project_path(), Some(PathBuf::from("/a")));
    }
}
