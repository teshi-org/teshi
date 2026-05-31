pub mod auth;
pub mod desktop;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "teshi",
    version,
    about = "Terminal-first BDD editor with AI assistance",
    long_about = "Terminal UI: `teshi` opens an empty buffer; `teshi .` recursively scans the current directory.\n\
                  `teshi path/` scans one level; add `--recursive` to scan subdirectories.\n\
                  Browser GUI: `teshi web [--project PATH]`.\n\
                  Native locator workflow: `teshi desktop [--project PATH]`.\n\
                  Headless CI runs: `teshi run [PATH] [--scenario NAME]`."
)]
pub struct Cli {
    /// Subcommands (auth, web, desktop, run)
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Recursively scan subdirectories for `.feature` files (TUI mode)
    #[arg(long, short = 'R')]
    pub recursive: bool,

    /// File or directory paths for TUI mode (`teshi .` = recursive project root)
    #[arg(value_name = "PATH")]
    pub paths: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage API credentials
    Auth {
        #[command(subcommand)]
        action: AuthCommand,
    },
    /// Browser GUI via loopback HTTP server (same UI as desktop, no Tauri install)
    Web {
        #[command(flatten)]
        options: teshi_web::WebOptions,
    },
    /// Native desktop shell (Chrome extension locator, embedded terminal)
    Desktop {
        /// Project directory to open on startup
        #[arg(long)]
        project: Option<String>,
        /// Project directory (shortcut for `--project`)
        #[arg(value_name = "PATH")]
        path: Option<String>,
    },
    /// Run BDD features headlessly (CI / scripts; streams NDJSON runner events)
    Run {
        /// Feature file or project directory (default: current directory)
        #[arg(value_name = "PATH")]
        path: Option<String>,
        /// Specific scenario name to run
        #[arg(long)]
        scenario: Option<String>,
        /// Runner binary / command
        #[arg(long)]
        runner_cmd: Option<String>,
        /// Runner additional arguments (repeatable)
        #[arg(long)]
        runner_arg: Option<Vec<String>>,
        /// Runner working directory
        #[arg(long)]
        runner_cwd: Option<String>,
        /// Deprecated alias for positional `PATH`
        #[arg(long, hide = true)]
        feature: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Store an API key for a provider
    Login {
        /// Provider name (e.g. openai, deepseek, ollama)
        #[arg(long, short)]
        provider: Option<String>,
    },
    /// List stored providers (without showing keys)
    List,
    /// Remove a stored provider credential
    Remove {
        /// Provider name to remove
        provider: String,
    },
    /// Show configuration and connectivity status
    Status,
    /// Migrate API keys from environment variables to secure storage
    Migrate,
}

impl Command {
    /// Builds run options from the `run` subcommand, resolving deprecated flags.
    pub fn run_options(
        path: Option<String>,
        scenario: Option<String>,
        runner_cmd: Option<String>,
        runner_arg: Option<Vec<String>>,
        runner_cwd: Option<String>,
        feature: Option<String>,
    ) -> crate::runner::RunCliOptions {
        let path = path.or(feature).map(PathBuf::from);
        crate::runner::RunCliOptions {
            path,
            scenario,
            runner_cmd,
            runner_args: runner_arg.unwrap_or_default(),
            runner_cwd: runner_cwd.map(PathBuf::from),
        }
    }
}
