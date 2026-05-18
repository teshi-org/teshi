pub mod auth;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "teshi", version, about = "Terminal-first mindmap editor")]
pub struct Cli {
    /// Subcommands (auth, run)
    #[command(subcommand)]
    pub command: Option<Command>,

    /// File paths or directories for TUI mode
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
    /// Run test features in CLI mode
    Run {
        /// Path to a .feature file
        #[arg(long)]
        feature: Option<String>,
        /// Specific scenario name to run
        #[arg(long)]
        scenario: Option<String>,
        /// Runner binary / command
        #[arg(long)]
        runner_cmd: Option<String>,
        /// Runner additional arguments (space-separated)
        #[arg(long)]
        runner_arg: Option<Vec<String>>,
        /// Runner working directory
        #[arg(long)]
        runner_cwd: Option<String>,
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
