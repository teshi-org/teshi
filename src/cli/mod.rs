pub mod auth;
pub mod browser;
pub mod desktop;
pub mod steps;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

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
    /// Subcommands (auth, web, desktop, run, steps, browser)
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
    /// Manage recorded Gherkin step bindings
    Steps {
        #[command(subcommand)]
        action: StepsCommand,
    },
    /// Inspect and execute locators through the browser bridge
    Browser {
        #[command(subcommand)]
        action: BrowserCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum StepsCommand {
    /// Write a pending locator proposal for the selected active step
    Propose(StepsProposeArgs),
    /// Confirm the pending proposal and persist it to step-bindings
    Confirm(StepsConfirmArgs),
    /// Reject the pending proposal
    Reject,
    /// Wait until the pending proposal is confirmed or rejected
    Wait(StepsWaitArgs),
    /// Resolve confirmed bindings for a feature
    Resolve(StepsResolveArgs),
    /// List persisted bindings and pending status for a feature
    List(StepsListArgs),
}

#[derive(Debug, Args)]
pub struct StepsProposeArgs {
    /// Candidate selector strategy, e.g. css
    #[arg(long, default_value = "css")]
    pub strategy: String,
    /// Candidate selector value; optional for navigate actions
    #[arg(long, alias = "selector")]
    pub value: Option<String>,
    /// Action to execute during replay
    #[arg(long, default_value = "click")]
    pub action: String,
    /// Optional input value for fill/assert_text/select/press_key/navigate
    #[arg(long)]
    pub value_arg: Option<String>,
    /// Candidate rank
    #[arg(long, default_value_t = 1)]
    pub rank: u32,
    /// Confidence in the selector, from 0.0 to 1.0
    #[arg(long, default_value_t = 0.8)]
    pub confidence: f64,
    /// Rationale shown in Desktop/web before confirmation
    #[arg(long, default_value = "Proposed by terminal agent")]
    pub rationale: String,
    /// Whether the primary candidate was highlighted successfully
    #[arg(long, default_value_t = false)]
    pub highlight_applied: bool,
}

#[derive(Debug, Args)]
pub struct StepsConfirmArgs {
    /// Candidate rank to persist
    #[arg(long, default_value_t = 1)]
    pub rank: u32,
    /// Override selector value before persisting
    #[arg(long, alias = "selector")]
    pub value: Option<String>,
}

#[derive(Debug, Args)]
pub struct StepsWaitArgs {
    /// Terminal state to wait for
    #[arg(long, value_enum, default_value = "either")]
    pub until: WaitUntilArg,
    /// Timeout in seconds
    #[arg(long, default_value_t = 120)]
    pub timeout: u64,
}

#[derive(Debug, Args)]
pub struct StepsResolveArgs {
    /// Feature path relative to the project root; defaults to active step feature
    #[arg(long)]
    pub feature: Option<String>,
    /// Only include bindings up to this source line
    #[arg(long)]
    pub until_line: Option<usize>,
}

#[derive(Debug, Args)]
pub struct StepsListArgs {
    /// Feature path relative to the project root; defaults to active step feature
    #[arg(long)]
    pub feature: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum WaitUntilArg {
    Confirmed,
    Rejected,
    Either,
}

#[derive(Debug, Subcommand)]
pub enum BrowserCommand {
    /// Read page accessibility and interactive element snapshot
    Snapshot(BrowserSnapshotArgs),
    /// Navigate the active browser tab to an explicit URL
    Navigate(BrowserNavigateArgs),
    /// Highlight a selector in the active browser
    Highlight(BrowserSelectorArgs),
    /// Clear active browser highlight
    ClearHighlight,
    /// Execute one locator action in the active browser
    Execute(BrowserExecuteArgs),
    /// Replay confirmed step bindings
    Replay(BrowserReplayArgs),
}

#[derive(Debug, Args)]
pub struct BrowserSnapshotArgs {
    /// Timeout in milliseconds waiting for the sidecar response
    #[arg(long, default_value_t = 60_000)]
    pub timeout_ms: u64,
}

#[derive(Debug, Args)]
pub struct BrowserNavigateArgs {
    /// URL to open in the active browser tab
    pub url: String,
    /// Timeout in milliseconds
    #[arg(long, default_value_t = 15000)]
    pub timeout_ms: u64,
}

#[derive(Debug, Args)]
pub struct BrowserSelectorArgs {
    /// CSS selector to highlight
    pub selector: String,
}

#[derive(Debug, Args)]
pub struct BrowserExecuteArgs {
    /// CSS selector to execute against
    #[arg(long)]
    pub selector: String,
    /// Action to execute
    #[arg(long, default_value = "click")]
    pub action: String,
    /// Optional input value for fill/assert_text/select/press_key
    #[arg(long)]
    pub value_arg: Option<String>,
    /// Timeout in milliseconds
    #[arg(long, default_value_t = 5000)]
    pub timeout_ms: u64,
}

#[derive(Debug, Args)]
pub struct BrowserReplayArgs {
    /// Feature path relative to project root; defaults to active step feature
    #[arg(long)]
    pub feature: Option<String>,
    /// Only replay bindings up to this source line
    #[arg(long)]
    pub until_line: Option<usize>,
    /// Print planned actions without executing them
    #[arg(long)]
    pub dry_run: bool,
    /// Replay without prompting between steps
    #[arg(long)]
    pub non_interactive: bool,
    /// Alias for --non-interactive
    #[arg(long)]
    pub yes: bool,
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[test]
    fn cli_command_factory_has_valid_args() {
        Cli::command().debug_assert();
    }

    #[test]
    fn browser_execute_accepts_selector_and_value_arg() {
        let cli = Cli::try_parse_from([
            "teshi",
            "browser",
            "execute",
            "--selector",
            "input[name=email]",
            "--action",
            "fill",
            "--value-arg",
            "test@example.com",
        ])
        .expect("parse browser execute");
        let Some(Command::Browser {
            action: BrowserCommand::Execute(args),
        }) = cli.command
        else {
            panic!("expected browser execute subcommand");
        };
        assert_eq!(args.selector, "input[name=email]");
        assert_eq!(args.action, "fill");
        assert_eq!(args.value_arg.as_deref(), Some("test@example.com"));
    }

    #[test]
    fn browser_execute_rejects_legacy_value_flag_for_action_input() {
        let err = Cli::try_parse_from([
            "teshi",
            "browser",
            "execute",
            "--selector",
            "h1",
            "--action",
            "assert_text",
            "--value",
            "Welcome",
        ])
        .expect_err("legacy --value should not parse as action input");
        let msg = err.to_string();
        assert!(
            msg.contains("value") || msg.contains("unexpected"),
            "unexpected error: {msg}"
        );
    }
}
