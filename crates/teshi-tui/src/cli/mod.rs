pub mod api;
pub mod auth;
pub mod browser;
pub mod browser_endpoint;
pub mod daemon;
pub mod desktop;
pub mod export;
pub mod install_skill;
pub mod locator_verify;
pub mod mcp;
pub mod replay_screenshots;
pub mod requirements;
pub mod steps;
pub mod terminal;
pub mod trace;
pub mod winapp;

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
    /// Subcommands (auth, web, desktop, run, steps, browser, winapp)
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Recursively scan subdirectories for `.feature` files (TUI mode)
    #[arg(long, short = 'R')]
    pub recursive: bool,

    /// File or directory paths for TUI mode (`teshi .` = recursive project root)
    #[arg(value_name = "PATH")]
    pub paths: Vec<String>,

    /// Override the user-level requirement store root for this process
    #[arg(long, value_name = "PATH", global = true)]
    pub requirements_root: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage API credentials
    Auth {
        #[command(subcommand)]
        action: AuthCommand,
    },
    /// Native desktop shell (Chrome extension locator, embedded terminal)
    Desktop {
        /// Project directory to open on startup
        #[arg(long)]
        project: Option<String>,
        /// Project directory (shortcut for `--project`)
        #[arg(value_name = "PATH")]
        path: Option<String>,
        /// Auto-start embedded browser on startup
        #[arg(long)]
        start_embedded: bool,
    },
    /// Inspect and manage the user-level requirement library
    Requirements {
        #[command(subcommand)]
        action: RequirementsCommand,
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
        action: Box<BrowserCommand>,
    },
    /// Serve Teshi's local agent integrations through Model Context Protocol
    Mcp {
        #[command(subcommand)]
        action: McpCommand,
    },
    /// Copy bundled agent skills into ~/.agents and link Agent discovery paths
    InstallSkill {
        /// Print the install plan without writing files
        #[arg(long)]
        dry_run: bool,
        /// Skip the confirmation prompt (required when stdin is not a TTY)
        #[arg(long)]
        yes: bool,
    },
    /// Inspect and execute locators through the WinUI3 bridge
    #[command(name = "winapp", alias = "win-app")]
    WinApp {
        #[command(subcommand)]
        action: WinAppCommand,
    },
    /// Start and inspect the HTTP API BDD sidecar
    Api {
        #[command(subcommand)]
        action: ApiCommand,
    },
    /// Control an interactive terminal via the terminal sidecar
    Terminal {
        #[command(subcommand)]
        action: TerminalCommand,
    },
    /// Export confirmed step-bindings to an external test project
    Export {
        #[command(flatten)]
        args: ExportArgs,
    },
    /// Manage the project daemon
    Daemon {
        #[command(subcommand)]
        action: DaemonCommand,
    },
    /// Record browser interactions via the engine (JS-injected capture)
    Record {
        /// Starting URL (default: https://github.com)
        #[arg(long, default_value = "https://github.com")]
        url: String,
        /// Feature path to associate with the recording
        #[arg(long)]
        feature: Option<String>,
        /// Auto-propose recorded steps as pending locator proposals
        #[arg(long)]
        auto_propose: bool,
    },
    /// Generate code artifacts via the engine (PageObject, step defs, project)
    Generate {
        #[command(subcommand)]
        action: GenerateCommand,
    },
    /// List and inspect exploration traces
    Trace {
        #[command(subcommand)]
        action: TraceCommand,
    },
}

#[derive(Debug, Args)]
pub struct ExportArgs {
    /// Export target (currently `behave` only)
    #[arg(long, value_enum, default_value = "behave")]
    pub target: ExportTargetArg,
    /// Feature path relative to project root
    #[arg(long)]
    pub feature: String,
    /// Output directory for the generated project
    #[arg(long, short = 'o', default_value = "tests-e2e")]
    pub out: String,
    /// Include Page Object modules under `pages/`
    #[arg(long, default_value_t = true)]
    pub with_po: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ExportTargetArg {
    Behave,
}

/// Subcommands for `teshi requirements`.
#[derive(Debug, Subcommand)]
pub enum RequirementsCommand {
    /// Print the resolved user-level requirement store path
    Path,
    /// Import a project's legacy `requirements/` directory into the current store
    ImportProject {
        /// Project directory (default: current directory)
        #[arg(value_name = "PROJECT")]
        project: Option<PathBuf>,
        /// Show the import plan without writing files
        #[arg(long)]
        dry_run: bool,
        /// Apply a conflict remapping plan without an interactive prompt
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum StepsCommand {
    /// List indexed step catalog with reuse statistics
    Catalog(StepsCatalogArgs),
    /// Set the active Gherkin step for locator recording
    Select(StepsSelectArgs),
    /// List steps that still need confirmed bindings
    Unbound(StepsFeatureArgs),
    /// Select the next unbound step and write active-step.json
    NextUnbound(StepsFeatureArgs),
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
    /// Remove a confirmed binding for one step line
    Unbind(StepsUnbindArgs),
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
    /// Require active-step.json line to match before proposing
    #[arg(long)]
    pub line: Option<usize>,
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
    /// On timeout, auto-confirm pending proposal (or reject on step mismatch)
    #[arg(long)]
    pub auto_confirm: bool,
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

#[derive(Debug, Args)]
pub struct StepsSelectArgs {
    /// Feature path relative to the project root
    #[arg(long)]
    pub feature: String,
    /// 1-based source line of the Gherkin step
    #[arg(long)]
    pub line: usize,
}

#[derive(Debug, Args)]
pub struct StepsUnbindArgs {
    /// Feature path relative to the project root
    #[arg(long)]
    pub feature: String,
    /// 1-based source line of the Gherkin step
    #[arg(long)]
    pub line: usize,
}

#[derive(Debug, Args)]
pub struct StepsFeatureArgs {
    /// Feature path relative to the project root; defaults to active step feature
    #[arg(long)]
    pub feature: Option<String>,
}

#[derive(Debug, Args)]
pub struct StepsCatalogArgs {
    /// Project root directory (default: current directory)
    #[arg(long)]
    pub project_root: Option<String>,
    /// Minimum reuse count to include
    #[arg(long)]
    pub min_count: Option<usize>,
    /// Maximum number of entries to return
    #[arg(long)]
    pub top: Option<usize>,
    /// Omit location details from output
    #[arg(long)]
    pub no_locations: bool,
    /// Output format (json or text)
    #[arg(long, default_value = "json")]
    pub format: String,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum WaitUntilArg {
    Confirmed,
    Rejected,
    Either,
}

#[derive(Debug, Subcommand)]
pub enum BrowserCommand {
    /// List registered browser-profile sessions and health
    Sessions,
    /// List windows and tabs for one browser-profile session
    Tabs(BrowserSessionArgs),
    /// Look up browser Profiles by opaque ID, label, browser name, or tab ID
    Lookup(BrowserLookupArgs),
    /// Set or clear a unique live Profile label
    ProfileLabel {
        #[command(subcommand)]
        action: BrowserProfileLabelCommand,
    },
    /// Mutate tabs, windows, and optional tab groups under a Profile lease
    Tab {
        #[command(subcommand)]
        action: BrowserTabCommand,
    },
    /// Acquire, renew, or release an exclusive browser-session lease
    Lease {
        #[command(subcommand)]
        action: BrowserLeaseCommand,
    },
    /// Manage explicit short-lived privileged browser capability grants
    Grant {
        #[command(subcommand)]
        action: BrowserGrantCommand,
    },
    /// List bounded metadata-only privileged audit records
    Audit(BrowserAuditArgs),
    /// Execute bounded arbitrary JavaScript under an explicit grant
    Javascript(BrowserJavascriptArgs),
    /// Execute a policy-allowlisted page-scoped raw CDP method
    Cdp(BrowserCdpArgs),
    /// List Cookies scoped to the selected tab (values need a second grant)
    Cookies(BrowserCookiesArgs),
    /// Read or set one allowlisted setting for the selected tab origin
    ContentSetting(BrowserContentSettingArgs),
    /// List bounded extension metadata (mutations remain disabled)
    Extensions(BrowserExtensionsArgs),
    /// Read page accessibility and interactive element snapshot
    Snapshot(BrowserSnapshotArgs),
    /// Navigate the active browser tab to an explicit URL
    Navigate(BrowserNavigateArgs),
    /// Highlight a selector in the active browser
    Highlight(BrowserSelectorArgs),
    /// Clear active browser highlight
    ClearHighlight(BrowserTargetArgs),
    /// Execute one locator action in the active browser
    Execute(Box<BrowserExecuteArgs>),
    /// Replay confirmed step bindings
    Replay(BrowserReplayArgs),
    /// Start headless Playwright sidecar for CI/scripts (writes `.teshi/cdp-endpoint.json`)
    ServeEmbedded(BrowserServeEmbeddedArgs),
    /// Check sidecar health (TCP + snapshot probe)
    Doctor,
    /// Restart embedded sidecar and refresh `.teshi/cdp-endpoint.json`
    Reconnect(BrowserReconnectArgs),
    /// Highlight + execute a locator and append verification log (RVP R4–R5)
    Verify(BrowserVerifyArgs),
    /// Probe an element for the best-priority locator (testid > role > label > ...)
    Enhance(BrowserSelectorArgs),
    /// Execute a locator with automatic self-healing retry chain
    HealExecute(BrowserExecuteArgs),
    /// Generate and verify ranked Playwright locator candidates
    Locator(BrowserLocatorArgs),
    /// Re-verify one structured Playwright locator candidate
    LocatorVerify(BrowserLocatorVerifyArgs),
    /// Capture screenshot evidence tied to a target and page revision
    Evidence(BrowserEvidenceArgs),
    /// Capture a viewport screenshot into managed artifact storage
    Screenshot(BrowserScreenshotArgs),
    /// Generate a PDF into managed artifact storage
    Pdf(BrowserPdfArgs),
    /// Capture bounded console diagnostics (requires p1.observability_artifacts)
    Console {
        #[command(subcommand)]
        action: BrowserConsoleCommand,
    },
    /// Capture bounded network metadata and explicit response bodies (requires P1)
    Network {
        #[command(subcommand)]
        action: BrowserNetworkCommand,
    },
    /// Explicitly remove managed browser artifact files
    ArtifactCleanup(BrowserArtifactCleanupArgs),
}

#[derive(Debug, Args, Default)]
pub struct BrowserTargetArgs {
    /// Opaque browser extension session identifier
    #[arg(long)]
    pub session: Option<String>,
    /// Browser-local window identifier
    #[arg(long, requires = "session")]
    pub window: Option<i64>,
    /// Browser-local tab identifier
    #[arg(long, requires = "session")]
    pub tab: Option<i64>,
    /// Exclusive browser-session lease token
    #[arg(long, requires = "session", allow_hyphen_values = true)]
    pub lease_token: Option<String>,
}

#[derive(Debug, Args)]
pub struct BrowserSessionArgs {
    /// Opaque browser extension session identifier
    #[arg(long)]
    pub session: String,
}

#[derive(Debug, Args)]
pub struct BrowserLookupArgs {
    #[arg(long)]
    pub session: Option<String>,
    #[arg(long)]
    pub label: Option<String>,
    #[arg(long)]
    pub browser_name: Option<String>,
    #[arg(long)]
    pub tab: Option<i64>,
}

#[derive(Debug, Subcommand)]
pub enum BrowserProfileLabelCommand {
    Set(BrowserProfileLabelSetArgs),
    Clear(BrowserSessionArgs),
}

#[derive(Debug, Args)]
pub struct BrowserProfileLabelSetArgs {
    #[arg(long)]
    pub session: String,
    #[arg(long)]
    pub label: String,
}

#[derive(Debug, Subcommand)]
pub enum BrowserTabCommand {
    Open(BrowserTabOpenArgs),
    Close(BrowserTargetArgs),
    Activate(BrowserTabActivateArgs),
    NewWindow(BrowserNewWindowArgs),
    Group(BrowserTabGroupArgs),
}

#[derive(Debug, Args)]
pub struct BrowserTabOpenArgs {
    pub url: String,
    #[arg(long)]
    pub active: bool,
    #[command(flatten)]
    pub target: BrowserTargetArgs,
}

#[derive(Debug, Args)]
pub struct BrowserTabActivateArgs {
    #[arg(long)]
    pub focus_window: bool,
    #[command(flatten)]
    pub target: BrowserTargetArgs,
}

#[derive(Debug, Args)]
pub struct BrowserNewWindowArgs {
    pub url: String,
    #[arg(long)]
    pub focused: bool,
    #[command(flatten)]
    pub target: BrowserTargetArgs,
}

#[derive(Debug, Args)]
pub struct BrowserTabGroupArgs {
    #[arg(long = "tab-id", required = true)]
    pub tab_ids: Vec<i64>,
    #[arg(long)]
    pub title: Option<String>,
    #[command(flatten)]
    pub target: BrowserTargetArgs,
}

#[derive(Debug, Subcommand)]
pub enum BrowserLeaseCommand {
    /// Acquire an exclusive browser-profile lease
    Acquire(BrowserLeaseAcquireArgs),
    /// Renew a matching browser-profile lease
    Renew(BrowserLeaseRenewArgs),
    /// Release a matching browser-profile lease
    Release(BrowserLeaseReleaseArgs),
}

#[derive(Debug, Subcommand)]
pub enum BrowserGrantCommand {
    /// Create a target-, project-, caller-, user-, and broker-bound grant
    Create(BrowserGrantCreateArgs),
    /// List active grant metadata without reusable secret tokens
    List(BrowserGrantListArgs),
    /// Revoke a grant by its public identifier
    Revoke(BrowserGrantRevokeArgs),
    /// Remove grants whose bounded lifetime has elapsed
    Expire,
}

#[derive(Debug, Args)]
pub struct BrowserGrantCreateArgs {
    #[arg(long, value_parser = ["javascript", "raw-cdp", "cookies", "cookie-values", "content-settings", "extension-management"])]
    pub capability: String,
    #[arg(long, default_value_t = 300)]
    pub ttl: u64,
    /// Confirm an interactive grant after reviewing its exact capability
    #[arg(long, conflicts_with = "non_interactive")]
    pub yes: bool,
    /// Request a policy-gated non-interactive grant
    #[arg(long)]
    pub non_interactive: bool,
    /// Exact capability acknowledgement required with --non-interactive
    #[arg(long, requires = "non_interactive")]
    pub acknowledge_capability: Option<String>,
    #[command(flatten)]
    pub target: BrowserTargetArgs,
}

#[derive(Debug, Args)]
pub struct BrowserGrantListArgs {
    #[arg(long)]
    pub session: Option<String>,
}

#[derive(Debug, Args)]
pub struct BrowserGrantRevokeArgs {
    pub grant_id: String,
}

#[derive(Debug, Args)]
pub struct BrowserAuditArgs {
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u64).range(1..=1000))]
    pub limit: u64,
}

#[derive(Debug, Args)]
pub struct BrowserJavascriptArgs {
    #[arg(long, group = "javascript_source", required = true)]
    pub expression: Option<String>,
    #[arg(long, group = "javascript_source")]
    pub file: Option<PathBuf>,
    #[arg(long)]
    pub grant_token: String,
    #[arg(long)]
    pub page_revision: Option<String>,
    #[arg(long, default_value_t = 5000)]
    pub timeout_ms: u64,
    #[arg(long, default_value_t = 65_536, value_parser = clap::value_parser!(u64).range(1..=1_048_576))]
    pub max_result_bytes: u64,
    #[command(flatten)]
    pub target: BrowserTargetArgs,
}

#[derive(Debug, Args)]
pub struct BrowserCdpArgs {
    pub method: String,
    #[arg(long, group = "cdp_params")]
    pub params_json: Option<String>,
    #[arg(long, group = "cdp_params")]
    pub params_file: Option<PathBuf>,
    #[arg(long)]
    pub grant_token: String,
    #[arg(long)]
    pub page_revision: Option<String>,
    #[arg(long, default_value_t = 65_536, value_parser = clap::value_parser!(u64).range(1..=1_048_576))]
    pub max_result_bytes: u64,
    #[command(flatten)]
    pub target: BrowserTargetArgs,
}

#[derive(Debug, Args)]
pub struct BrowserCookiesArgs {
    /// Metadata-access grant token (`cookies`)
    #[arg(long)]
    pub grant_token: String,
    /// Include values; requires --value-grant-token for `cookie-values`
    #[arg(long, requires = "value_grant_token")]
    pub include_values: bool,
    /// Separate value-access grant token (`cookie-values`)
    #[arg(long, requires = "include_values")]
    pub value_grant_token: Option<String>,
    #[arg(long, default_value_t = 200, value_parser = clap::value_parser!(u64).range(1..=500))]
    pub max_entries: u64,
    #[arg(long, default_value_t = 262_144, value_parser = clap::value_parser!(u64).range(1..=1_048_576))]
    pub max_result_bytes: u64,
    #[command(flatten)]
    pub target: BrowserTargetArgs,
}

#[derive(Debug, Args)]
pub struct BrowserContentSettingArgs {
    /// Allowlisted setting name (notifications, popups, geolocation, camera, microphone, automatic_downloads)
    pub setting: String,
    /// Set an origin-scoped value; omit to read (allow, block, ask)
    #[arg(long, value_parser = ["allow", "block", "ask"])]
    pub value: Option<String>,
    #[arg(long)]
    pub grant_token: String,
    #[command(flatten)]
    pub target: BrowserTargetArgs,
}

#[derive(Debug, Args)]
pub struct BrowserExtensionsArgs {
    #[arg(long)]
    pub grant_token: String,
    #[arg(long, default_value_t = 200, value_parser = clap::value_parser!(u64).range(1..=500))]
    pub max_entries: u64,
    #[command(flatten)]
    pub target: BrowserTargetArgs,
}

#[derive(Debug, Args)]
pub struct BrowserLeaseAcquireArgs {
    /// Opaque browser extension session identifier
    #[arg(long)]
    pub session: String,
    /// Display-only lease owner label
    #[arg(long, default_value = "teshi-cli")]
    pub owner: String,
    /// Requested lease lifetime in seconds (bounded by the broker)
    #[arg(long, default_value_t = 60)]
    pub ttl: u64,
}

#[derive(Debug, Args)]
pub struct BrowserLeaseRenewArgs {
    /// Opaque browser extension session identifier
    #[arg(long)]
    pub session: String,
    /// Secret lease token returned by acquisition
    #[arg(long, allow_hyphen_values = true)]
    pub lease_token: String,
    /// Requested lease lifetime in seconds (bounded by the broker)
    #[arg(long, default_value_t = 60)]
    pub ttl: u64,
}

#[derive(Debug, Args)]
pub struct BrowserLeaseReleaseArgs {
    /// Opaque browser extension session identifier
    #[arg(long)]
    pub session: String,
    /// Secret lease token returned by acquisition
    #[arg(long, allow_hyphen_values = true)]
    pub lease_token: String,
}

#[derive(Debug, Args)]
pub struct BrowserSnapshotArgs {
    /// Timeout in milliseconds waiting for the sidecar response
    #[arg(long, default_value_t = 60_000)]
    pub timeout_ms: u64,
    #[command(flatten)]
    pub target: BrowserTargetArgs,
}

#[derive(Debug, Args)]
pub struct BrowserNavigateArgs {
    /// URL to open in the active browser tab
    pub url: String,
    /// Timeout in milliseconds
    #[arg(long, default_value_t = 15000)]
    pub timeout_ms: u64,
    /// Return bounded before/after page summaries and a structured diff
    #[arg(long)]
    pub monitor: bool,
    #[command(flatten)]
    pub target: BrowserTargetArgs,
}

#[derive(Debug, Args)]
pub struct BrowserSelectorArgs {
    /// CSS selector to highlight
    pub selector: String,
    #[command(flatten)]
    pub target: BrowserTargetArgs,
}

#[derive(Debug, Args)]
pub struct BrowserExecuteArgs {
    /// CSS compatibility selector (mutually exclusive with --reference/--candidate-json)
    #[arg(long, group = "browser_element_input")]
    pub selector: Option<String>,
    /// Revision-bound compact snapshot reference such as @e1
    #[arg(long, group = "browser_element_input")]
    pub reference: Option<String>,
    /// Structured Playwright locator candidate JSON
    #[arg(long, group = "browser_element_input")]
    pub candidate_json: Option<String>,
    /// Snapshot identity required when constraining a compact reference
    #[arg(long)]
    pub snapshot_id: Option<String>,
    /// Expected page-context revision
    #[arg(long)]
    pub page_revision: Option<String>,
    /// Action to execute
    #[arg(long, default_value = "click")]
    pub action: String,
    /// Optional input value for fill/assert_text/select/press_key
    #[arg(long)]
    pub value_arg: Option<String>,
    /// Explicit project-authorized local file to upload (repeatable; upload action only)
    #[arg(long = "file")]
    pub files: Vec<PathBuf>,
    /// Timeout in milliseconds
    #[arg(long, default_value_t = 5000)]
    pub timeout_ms: u64,
    /// Wait until URL contains this value after the action
    #[arg(long, group = "browser_wait")]
    pub wait_url: Option<String>,
    /// Wait until visible page text contains this value
    #[arg(long, group = "browser_wait")]
    pub wait_text: Option<String>,
    /// Wait for the selected element state: visible, hidden, enabled, disabled
    #[arg(long, group = "browser_wait")]
    pub wait_state: Option<String>,
    /// Wait for the page revision to change from --page-revision
    #[arg(long, group = "browser_wait", requires = "page_revision")]
    pub wait_revision_change: bool,
    /// Wait for bounded document load completion
    #[arg(long, group = "browser_wait")]
    pub wait_load: bool,
    /// Allow pointer action to focus/activate the target browser window
    #[arg(long)]
    pub focus: bool,
    /// Return bounded before/after page summaries and a structured diff
    #[arg(long)]
    pub monitor: bool,
    #[command(flatten)]
    pub target: BrowserTargetArgs,
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
    #[command(flatten)]
    pub target: BrowserTargetArgs,
}

#[derive(Debug, Args)]
pub struct BrowserServeEmbeddedArgs {
    /// Project directory (default: current working directory)
    #[arg(long)]
    pub project: Option<PathBuf>,
    /// Navigate the embedded browser to this URL after the sidecar starts
    #[arg(long)]
    pub navigate: Option<String>,
}

#[derive(Debug, Args)]
pub struct BrowserReconnectArgs {
    /// Navigate after reconnect (default: last page_url from cdp-endpoint.json)
    #[arg(long)]
    pub navigate: Option<String>,
    /// Seconds to wait for the new sidecar (default 45)
    #[arg(long, default_value_t = 45)]
    pub wait_secs: u64,
}

#[derive(Debug, Args)]
pub struct BrowserVerifyArgs {
    /// Step line this verification applies to (for strict propose gate)
    #[arg(long)]
    pub step_line: u32,
    /// CSS selector
    #[arg(long)]
    pub selector: String,
    /// Locator action (must match a future `steps propose --action`)
    #[arg(long, default_value = "click")]
    pub action: String,
    /// Optional value for fill/type/assert_text/select/press_key
    #[arg(long)]
    pub value_arg: Option<String>,
    /// Timeout in milliseconds
    #[arg(long, default_value_t = 5000)]
    pub timeout_ms: u64,
    #[command(flatten)]
    pub target: BrowserTargetArgs,
}

#[derive(Debug, Args)]
pub struct BrowserLocatorArgs {
    #[command(flatten)]
    pub target: BrowserTargetArgs,
    /// Human-readable element purpose
    #[arg(long)]
    pub purpose: Option<String>,
    /// Expected visible or accessible text
    #[arg(long)]
    pub text: Option<String>,
    /// Expected accessible role
    #[arg(long)]
    pub role: Option<String>,
    /// Snapshot-local element reference
    #[arg(long)]
    pub element_ref: Option<String>,
    /// Selected Gherkin step text
    #[arg(long)]
    pub gherkin_step: Option<String>,
    /// Override a project test-id attribute (repeatable)
    #[arg(long = "test-id-attribute")]
    pub test_id_attributes: Vec<String>,
    /// Operation timeout in milliseconds
    #[arg(long, default_value_t = 60_000)]
    pub timeout_ms: u64,
}

#[derive(Debug, Args)]
pub struct BrowserLocatorVerifyArgs {
    #[command(flatten)]
    pub target: BrowserTargetArgs,
    /// Structured locator candidate JSON returned by `browser locator`
    #[arg(long)]
    pub candidate_json: String,
    /// Expected page-context revision
    #[arg(long)]
    pub page_revision: String,
    /// Operation timeout in milliseconds
    #[arg(long, default_value_t = 30_000)]
    pub timeout_ms: u64,
}

#[derive(Debug, Args)]
pub struct BrowserEvidenceArgs {
    #[command(flatten)]
    pub target: BrowserTargetArgs,
    /// Expected page-context revision
    #[arg(long)]
    pub page_revision: String,
    /// Operation timeout in milliseconds
    #[arg(long, default_value_t = 30_000)]
    pub timeout_ms: u64,
}

#[derive(Debug, Args)]
pub struct BrowserScreenshotArgs {
    #[command(flatten)]
    pub target: BrowserTargetArgs,
    /// Optional expected page-context revision
    #[arg(long)]
    pub page_revision: Option<String>,
    /// Revision-bound compact element reference
    #[arg(long)]
    pub reference: Option<String>,
    /// Structured Playwright locator candidate JSON
    #[arg(long)]
    pub candidate_json: Option<String>,
    /// CSS compatibility selector for an element screenshot
    #[arg(long)]
    pub selector: Option<String>,
    /// Snapshot identity constraining --reference
    #[arg(long)]
    pub snapshot_id: Option<String>,
    /// Image format: png or jpeg
    #[arg(long, default_value = "png")]
    pub format: String,
    /// JPEG quality from 0 to 100 (JPEG only)
    #[arg(long)]
    pub quality: Option<u8>,
    /// Capture the full scrollable page instead of only the viewport
    #[arg(long)]
    pub full_page: bool,
    /// Operation timeout in milliseconds
    #[arg(long, default_value_t = 30_000)]
    pub timeout_ms: u64,
}

#[derive(Debug, Args)]
pub struct BrowserPdfArgs {
    #[command(flatten)]
    pub target: BrowserTargetArgs,
    #[arg(long)]
    pub page_revision: Option<String>,
    /// Paper format such as A4, Letter, or Legal
    #[arg(long, default_value = "A4")]
    pub paper: String,
    #[arg(long)]
    pub landscape: bool,
    /// Render scale from 0.1 to 2.0
    #[arg(long, default_value_t = 1.0)]
    pub scale: f64,
    #[arg(long)]
    pub print_background: bool,
    #[arg(long, default_value_t = 30_000)]
    pub timeout_ms: u64,
}

#[derive(Debug, Args)]
pub struct BrowserArtifactCleanupArgs {
    /// Exact managed artifact path to remove (repeatable)
    #[arg(long = "path", required = true)]
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum BrowserConsoleCommand {
    /// Start target-scoped bounded console capture
    Start(BrowserConsoleStartArgs),
    /// List captured console events with optional tighter limits
    List(BrowserConsoleListArgs),
    /// Clear retained events while capture remains active
    Clear(BrowserTargetArgs),
    /// Stop capture and discard retained events
    Stop(BrowserTargetArgs),
}

#[derive(Debug, Args)]
pub struct BrowserConsoleStartArgs {
    #[command(flatten)]
    pub target: BrowserTargetArgs,
    /// Levels to retain: debug,log,info,warn,error (comma-separated or repeatable)
    #[arg(long, value_delimiter = ',')]
    pub level: Vec<String>,
    #[arg(long)]
    pub max_age_ms: Option<u64>,
    #[arg(long)]
    pub max_entries: Option<u64>,
    #[arg(long)]
    pub max_bytes: Option<u64>,
    /// Additional sensitive field name to redact (repeatable)
    #[arg(long = "sensitive-field")]
    pub sensitive_fields: Vec<String>,
}

#[derive(Debug, Args)]
pub struct BrowserConsoleListArgs {
    #[command(flatten)]
    pub target: BrowserTargetArgs,
    #[arg(long, value_delimiter = ',')]
    pub level: Vec<String>,
    #[arg(long)]
    pub max_age_ms: Option<u64>,
    #[arg(long)]
    pub max_entries: Option<u64>,
    #[arg(long)]
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Subcommand)]
pub enum BrowserNetworkCommand {
    /// Start target-scoped metadata-only network capture
    Start(BrowserNetworkStartArgs),
    /// List captured request/response metadata without bodies
    List(BrowserNetworkListArgs),
    /// Get one request's metadata and optionally its bounded response body
    Detail(BrowserNetworkDetailArgs),
    /// Clear retained request metadata while capture remains active
    Clear(BrowserTargetArgs),
    /// Stop capture and discard retained request metadata
    Stop(BrowserTargetArgs),
}

#[derive(Debug, Args)]
pub struct BrowserNetworkStartArgs {
    #[command(flatten)]
    pub target: BrowserTargetArgs,
    /// Exact hostname to capture (repeatable)
    #[arg(long = "host", required = true, value_parser = normalize_exact_hostname)]
    pub hosts: Vec<String>,
    /// Retain bounded raw request bodies for matching requests
    #[arg(long)]
    pub request_body: bool,
    /// Maximum bytes retained from each request body
    #[arg(long, requires = "request_body", value_parser = clap::value_parser!(u64).range(1..))]
    pub max_request_body_bytes: Option<u64>,
    #[arg(long)]
    pub max_age_ms: Option<u64>,
    #[arg(long)]
    pub max_entries: Option<u64>,
    #[arg(long)]
    pub max_bytes: Option<u64>,
    /// Maximum decoded response body bytes returned by an explicit detail request
    #[arg(long)]
    pub max_body_bytes: Option<u64>,
    /// Additional sensitive header or query-field name to redact (repeatable)
    #[arg(long = "sensitive-field")]
    pub sensitive_fields: Vec<String>,
}

/// Validates and canonicalizes one exact network-capture hostname.
///
/// # Errors
///
/// Returns an error when the input contains URL components, wildcards,
/// credentials, non-ASCII characters, or invalid DNS label syntax.
pub(crate) fn normalize_exact_hostname(value: &str) -> Result<String, String> {
    if value.is_empty() || value.trim() != value {
        return Err("hostname must not be empty or contain surrounding whitespace".into());
    }
    if value.contains("://") {
        return Err("hostname must not include a URL scheme".into());
    }
    if value.contains('@') {
        return Err("hostname must not include credentials".into());
    }
    if value.contains(':') {
        return Err("hostname must not include a port".into());
    }
    if value.chars().any(|character| "/\\?#".contains(character)) {
        return Err("hostname must not include a path, query, or fragment".into());
    }
    if value.contains('*') {
        return Err("hostname must not include a wildcard".into());
    }
    if !value.is_ascii() {
        return Err("hostname must contain only ASCII characters".into());
    }
    if value.ends_with("..") {
        return Err("hostname may have at most one trailing dot".into());
    }

    let normalized = value
        .strip_suffix('.')
        .unwrap_or(value)
        .to_ascii_lowercase();
    if normalized.is_empty() || normalized.len() > 253 {
        return Err("hostname must contain between 1 and 253 characters".into());
    }
    if normalized.split('.').any(|label| {
        label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    }) {
        return Err(
            "hostname labels must use 1-63 ASCII letters, digits, or interior hyphens".into(),
        );
    }
    Ok(normalized)
}

#[derive(Debug, Args)]
pub struct BrowserNetworkListArgs {
    #[command(flatten)]
    pub target: BrowserTargetArgs,
    #[arg(long)]
    pub max_age_ms: Option<u64>,
    #[arg(long)]
    pub max_entries: Option<u64>,
    #[arg(long)]
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Args)]
pub struct BrowserNetworkDetailArgs {
    /// CDP request identifier returned by `browser network list`
    pub network_request_id: String,
    #[command(flatten)]
    pub target: BrowserTargetArgs,
    /// Explicitly request the bounded response body; omitted by default
    #[arg(long)]
    pub include_body: bool,
    #[arg(long, requires = "include_body")]
    pub max_body_bytes: Option<u64>,
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// Serve the browser-agent tools over newline-delimited JSON-RPC on STDIO
    Serve(McpServeArgs),
}

#[derive(Debug, Args)]
pub struct McpServeArgs {
    /// Use standard input/output as the local MCP transport
    #[arg(long, default_value_t = false)]
    pub stdio: bool,
    /// Project root containing `.teshi/cdp-endpoint.json`
    #[arg(long)]
    pub project: Option<PathBuf>,
    /// Advertise safe P0 mutation tools (disabled by default)
    #[arg(long, default_value_t = false)]
    pub allow_browser_mutations: bool,
    /// Explicit P2 tool capability allowlist; each entry must also be allowed by policy
    #[arg(long = "allow-privileged-capability", value_parser = ["javascript", "raw-cdp", "cookies", "cookie-values", "content-settings", "extension-management"])]
    pub allow_privileged_capabilities: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum WinAppCommand {
    /// List visible top-level windows for attach
    ListWindows,
    /// Attach to an existing WinUI3/native window
    Attach(WinAppAttachArgs),
    /// Launch an app and attach to its first visible window
    Launch(WinAppLaunchArgs),
    /// Read UI Automation tree and interactive element snapshot
    Snapshot(WinAppSnapshotArgs),
    /// Highlight a UIA selector in the attached app
    Highlight(WinAppSelectorArgs),
    /// Clear active WinUI3 highlight
    ClearHighlight,
    /// Execute one locator action in the attached app
    Execute(WinAppExecuteArgs),
    /// Replay confirmed UIA step bindings
    Replay(WinAppReplayArgs),
}

/// HTTP API BDD sidecar (`teshi api`).
#[derive(Debug, Subcommand)]
pub enum ApiCommand {
    /// Start (or reuse) the loopback API sidecar and write `.teshi/api-endpoint.json`
    Serve(ApiServeArgs),
    /// Check sidecar health (`ping` / `doctor`)
    Doctor,
    /// Stop the sidecar recorded in `.teshi/api-endpoint.json`
    Stop,
    /// Fetch one stored HTTP exchange (redacted by default)
    Exchange(ApiExchangeArgs),
}

/// Arguments for `teshi api serve`.
#[derive(Debug, Args)]
pub struct ApiServeArgs {
    /// Project directory (default: current working directory)
    #[arg(long)]
    pub project: Option<PathBuf>,
}

/// Arguments for `teshi api exchange`.
#[derive(Debug, Args)]
pub struct ApiExchangeArgs {
    /// Exchange id from an `http_exchange` event
    pub id: String,
    /// Return unredacted headers and bodies (inspector expand)
    #[arg(long)]
    pub plaintext: bool,
}

#[derive(Debug, Args)]
pub struct WinAppAttachArgs {
    /// Native top-level window handle
    #[arg(long)]
    pub hwnd: Option<u64>,
    /// Case-insensitive title fragment
    #[arg(long)]
    pub title: Option<String>,
    /// Process identifier owning the target window
    #[arg(long)]
    pub pid: Option<u32>,
    /// Case-insensitive process executable name fragment
    #[arg(long)]
    pub process_name: Option<String>,
}

#[derive(Debug, Args)]
pub struct WinAppLaunchArgs {
    /// App executable path
    pub path: String,
    /// Optional title fragment to prefer after launch
    #[arg(long)]
    pub title: Option<String>,
    /// Timeout in milliseconds waiting for a visible window
    #[arg(long, default_value_t = 15000)]
    pub timeout_ms: u64,
    /// Arguments passed to the launched process
    #[arg(last = true)]
    pub args: Vec<String>,
}

#[derive(Debug, Args)]
pub struct WinAppSnapshotArgs {
    /// Timeout in milliseconds waiting for the sidecar response
    #[arg(long, default_value_t = 60_000)]
    pub timeout_ms: u64,
}

#[derive(Debug, Args)]
pub struct WinAppSelectorArgs {
    /// UIA selector, e.g. uia:automation_id=LoginButton
    pub selector: String,
}

#[derive(Debug, Args)]
pub struct WinAppExecuteArgs {
    /// UIA selector to execute against
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
    /// Input mode: foreground (default) or background (non-intrusive PostMessage)
    #[arg(long, default_value = "foreground")]
    pub mode: String,
}

#[derive(Debug, Args)]
pub struct WinAppReplayArgs {
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
    /// Launch this executable and attach before replay (when not already attached)
    #[arg(long)]
    pub launch: Option<String>,
    /// Input mode for all replay steps: foreground (default) or background (non-intrusive PostMessage)
    #[arg(long, default_value = "foreground")]
    pub mode: String,
}

#[derive(Debug, Subcommand)]
pub enum TerminalCommand {
    /// Start the terminal sidecar (blocking, press Ctrl+C to stop)
    ServeEmbedded,
    /// Read the current terminal screen as a structured JSON grid
    Snapshot,
    /// Query terminal process state (low-cost polling)
    Status,
    /// Execute a command and wait for completion
    Exec(TerminalExecArgs),
    /// Write text to the terminal stdin
    Send(TerminalSendArgs),
    /// Resize the terminal viewport
    Resize(TerminalResizeArgs),
    /// Kill the current terminal session
    Kill,
}

#[derive(Debug, Args)]
pub struct TerminalExecArgs {
    /// Command to execute in the shell
    pub command: String,
    /// Timeout in milliseconds (default: 60000)
    #[arg(long, default_value_t = 60_000)]
    pub timeout_ms: u64,
}

#[derive(Debug, Args)]
pub struct TerminalSendArgs {
    /// Text to write to the terminal
    pub text: String,
    /// Append a newline after the text
    #[arg(long)]
    pub newline: bool,
}

#[derive(Debug, Args)]
pub struct TerminalResizeArgs {
    /// Number of columns
    pub cols: u16,
    /// Number of rows
    pub rows: u16,
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

#[derive(Debug, Subcommand)]
pub enum DaemonCommand {
    /// Start the project daemon if not already running
    Start,
    /// Stop the running project daemon
    Stop,
}

#[derive(Debug, Subcommand)]
pub enum GenerateCommand {
    /// Generate PageObject from recorded steps or feature bindings
    Po {
        /// Path to scenario JSON file
        #[arg(value_name = "SCENARIO_JSON")]
        scenario: Option<String>,
        /// Feature path to read bindings from
        #[arg(long)]
        feature: Option<String>,
        /// Output file
        #[arg(long, short = 'o')]
        output: Option<String>,
    },
    /// Generate behave step definitions from a PageObject
    Steps {
        /// Path to PageObject .py file
        #[arg(value_name = "PO_FILE")]
        page_object: Option<String>,
        /// Feature path to read bindings from (generates PO first, then steps)
        #[arg(long)]
        feature: Option<String>,
        /// Output file
        #[arg(long, short = 'o')]
        output: Option<String>,
    },
    /// Generate a complete behave BDD project
    Project {
        /// Feature path relative to project root (required)
        #[arg(long)]
        feature: String,
        /// Output directory (default: generated)
        #[arg(long, short = 'o', default_value = "generated")]
        output: String,
    },
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

#[derive(Debug, Subcommand)]
pub enum TraceCommand {
    /// List all exploration traces
    List,
    /// Show details of a specific trace
    Show {
        /// Trace session ID (e.g. 'explore-1234567890')
        id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    fn browser_command(cli: Cli) -> BrowserCommand {
        let Some(Command::Browser { action }) = cli.command else {
            panic!("expected browser subcommand");
        };
        *action
    }

    #[test]
    fn cli_command_factory_has_valid_args() {
        Cli::command().debug_assert();
    }

    #[test]
    fn install_skill_parses_dry_run_and_yes() {
        let cli = Cli::try_parse_from(["teshi", "install-skill", "--dry-run", "--yes"])
            .expect("parse install-skill");
        let Some(Command::InstallSkill { dry_run, yes }) = cli.command else {
            panic!("expected install-skill subcommand");
        };
        assert!(dry_run);
        assert!(yes);
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
        let BrowserCommand::Execute(args) = browser_command(cli) else {
            panic!("expected browser execute subcommand");
        };
        assert_eq!(args.selector.as_deref(), Some("input[name=email]"));
        assert_eq!(args.action, "fill");
        assert_eq!(args.value_arg.as_deref(), Some("test@example.com"));
    }

    #[test]
    fn browser_execute_accepts_reference_pointer_and_typed_wait() {
        let cli = Cli::try_parse_from([
            "teshi",
            "browser",
            "execute",
            "--reference",
            "@e1",
            "--page-revision",
            "revision-a",
            "--snapshot-id",
            "snapshot-a",
            "--action",
            "pointer_click",
            "--wait-text",
            "Saved",
            "--focus",
            "--session",
            "profile-a",
            "--window",
            "7",
            "--tab",
            "42",
            "--lease-token",
            "lease-a",
        ])
        .expect("parse reference pointer action");
        let BrowserCommand::Execute(args) = browser_command(cli) else {
            panic!("expected browser execute subcommand");
        };
        assert_eq!(args.reference.as_deref(), Some("@e1"));
        assert_eq!(args.action, "pointer_click");
        assert_eq!(args.wait_text.as_deref(), Some("Saved"));
        assert!(args.focus);
    }

    #[test]
    fn browser_execute_accepts_monitoring_and_explicit_upload_files() {
        let cli = Cli::try_parse_from([
            "teshi",
            "browser",
            "execute",
            "--selector",
            "input[type=file]",
            "--action",
            "upload",
            "--file",
            "fixtures/avatar.png",
            "--file",
            "fixtures/profile.json",
            "--monitor",
        ])
        .expect("parse monitored upload action");
        let BrowserCommand::Execute(args) = browser_command(cli) else {
            panic!("expected browser execute subcommand");
        };
        assert_eq!(args.action, "upload");
        assert_eq!(args.files.len(), 2);
        assert!(args.monitor);
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

    #[test]
    fn browser_locator_accepts_explicit_target_lease_and_intent() {
        let cli = Cli::try_parse_from([
            "teshi",
            "browser",
            "locator",
            "--session",
            "profile-a",
            "--window",
            "7",
            "--tab",
            "42",
            "--lease-token",
            "lease-a",
            "--role",
            "button",
            "--text",
            "Save",
            "--test-id-attribute",
            "data-qa",
        ])
        .expect("parse browser locator");
        let BrowserCommand::Locator(args) = browser_command(cli) else {
            panic!("expected browser locator subcommand");
        };
        assert_eq!(args.target.session.as_deref(), Some("profile-a"));
        assert_eq!(args.target.window, Some(7));
        assert_eq!(args.target.tab, Some(42));
        assert_eq!(args.target.lease_token.as_deref(), Some("lease-a"));
        assert_eq!(args.role.as_deref(), Some("button"));
        assert_eq!(args.test_id_attributes, ["data-qa"]);
    }

    #[test]
    fn browser_lease_acquire_has_bounded_owner_inputs() {
        let cli = Cli::try_parse_from([
            "teshi",
            "browser",
            "lease",
            "acquire",
            "--session",
            "profile-b",
            "--owner",
            "agent-two",
            "--ttl",
            "45",
        ])
        .expect("parse browser lease acquire");
        let BrowserCommand::Lease {
            action: BrowserLeaseCommand::Acquire(args),
        } = browser_command(cli)
        else {
            panic!("expected browser lease acquire subcommand");
        };
        assert_eq!(args.session, "profile-b");
        assert_eq!(args.owner, "agent-two");
        assert_eq!(args.ttl, 45);
    }

    #[test]
    fn browser_locator_accepts_legacy_hyphen_leading_lease_token() {
        let cli = Cli::try_parse_from([
            "teshi",
            "browser",
            "locator",
            "--session",
            "profile-a",
            "--window",
            "7",
            "--tab",
            "42",
            "--lease-token",
            "-legacy-token",
            "--role",
            "button",
        ])
        .expect("parse a lease token beginning with a hyphen");
        let BrowserCommand::Locator(args) = browser_command(cli) else {
            panic!("expected browser locator subcommand");
        };
        assert_eq!(args.target.lease_token.as_deref(), Some("-legacy-token"));
    }

    #[test]
    fn browser_console_start_accepts_target_filters_and_limits() {
        let cli = Cli::try_parse_from([
            "teshi",
            "browser",
            "console",
            "start",
            "--session",
            "profile-a",
            "--window",
            "7",
            "--tab",
            "42",
            "--lease-token",
            "lease-a",
            "--level",
            "info,error",
            "--max-entries",
            "200",
            "--sensitive-field",
            "account-id",
        ])
        .expect("parse console capture start");
        let BrowserCommand::Console {
            action: BrowserConsoleCommand::Start(args),
        } = browser_command(cli)
        else {
            panic!("expected browser console start subcommand");
        };
        assert_eq!(args.level, ["info", "error"]);
        assert_eq!(args.max_entries, Some(200));
        assert_eq!(args.sensitive_fields, ["account-id"]);
    }

    #[test]
    fn browser_grant_create_requires_explicit_scope_inputs() {
        let cli = Cli::try_parse_from([
            "teshi",
            "browser",
            "grant",
            "create",
            "--capability",
            "javascript",
            "--yes",
            "--session",
            "profile-a",
            "--window",
            "7",
            "--tab",
            "42",
            "--lease-token",
            "lease-a",
        ])
        .expect("parse privileged grant create");
        let BrowserCommand::Grant {
            action: BrowserGrantCommand::Create(args),
        } = browser_command(cli)
        else {
            panic!("expected browser grant create");
        };
        assert_eq!(args.capability, "javascript");
        assert!(args.yes);
        assert_eq!(args.target.session.as_deref(), Some("profile-a"));
    }

    #[test]
    fn browser_cookie_values_require_distinct_value_grant() {
        let cli = Cli::try_parse_from([
            "teshi",
            "browser",
            "cookies",
            "--grant-token",
            "metadata-grant",
            "--include-values",
            "--value-grant-token",
            "value-grant",
            "--session",
            "profile-a",
            "--window",
            "7",
            "--tab",
            "42",
            "--lease-token",
            "lease-a",
        ])
        .expect("parse privileged Cookie listing");
        let BrowserCommand::Cookies(args) = browser_command(cli) else {
            panic!("expected browser cookies subcommand");
        };
        assert!(args.include_values);
        assert_eq!(args.value_grant_token.as_deref(), Some("value-grant"));
    }

    #[test]
    fn browser_privileged_metadata_commands_parse_scoped_inputs() {
        let setting = Cli::try_parse_from([
            "teshi",
            "browser",
            "content-setting",
            "notifications",
            "--value",
            "block",
            "--grant-token",
            "setting-grant",
            "--session",
            "profile-a",
            "--window",
            "7",
            "--tab",
            "42",
            "--lease-token",
            "lease-a",
        ])
        .expect("parse content setting command");
        let BrowserCommand::ContentSetting(args) = browser_command(setting) else {
            panic!("expected content-setting subcommand");
        };
        assert_eq!(args.setting, "notifications");
        assert_eq!(args.value.as_deref(), Some("block"));

        let extensions = Cli::try_parse_from([
            "teshi",
            "browser",
            "extensions",
            "--grant-token",
            "management-grant",
            "--session",
            "profile-a",
            "--window",
            "7",
            "--tab",
            "42",
            "--lease-token",
            "lease-a",
        ])
        .expect("parse extension metadata command");
        let BrowserCommand::Extensions(args) = browser_command(extensions) else {
            panic!("expected extensions subcommand");
        };
        assert_eq!(args.max_entries, 200);
    }

    #[test]
    fn browser_network_detail_requires_explicit_body_flag_for_body_limit() {
        let cli = Cli::try_parse_from([
            "teshi",
            "browser",
            "network",
            "detail",
            "request-1",
            "--session",
            "profile-a",
            "--window",
            "7",
            "--tab",
            "42",
            "--lease-token",
            "lease-a",
            "--include-body",
            "--max-body-bytes",
            "65536",
        ])
        .expect("parse network detail");
        let BrowserCommand::Network {
            action: BrowserNetworkCommand::Detail(args),
        } = browser_command(cli)
        else {
            panic!("expected browser network detail subcommand");
        };
        assert_eq!(args.network_request_id, "request-1");
        assert!(args.include_body);
        assert_eq!(args.max_body_bytes, Some(65_536));

        Cli::try_parse_from([
            "teshi",
            "browser",
            "network",
            "detail",
            "request-1",
            "--session",
            "profile-a",
            "--window",
            "7",
            "--tab",
            "42",
            "--lease-token",
            "lease-a",
            "--max-body-bytes",
            "65536",
        ])
        .expect_err("body limit without --include-body must fail");
    }

    #[test]
    fn browser_network_start_requires_and_normalizes_repeatable_exact_hosts() {
        let cli = Cli::try_parse_from([
            "teshi",
            "browser",
            "network",
            "start",
            "--host",
            "API.Example.Test.",
            "--host",
            "uploads.example.test",
            "--request-body",
            "--max-request-body-bytes",
            "65536",
        ])
        .expect("parse filtered network capture");
        let BrowserCommand::Network {
            action: BrowserNetworkCommand::Start(args),
        } = browser_command(cli)
        else {
            panic!("expected browser network start subcommand");
        };
        assert_eq!(args.hosts, ["api.example.test", "uploads.example.test"]);
        assert!(args.request_body);
        assert_eq!(args.max_request_body_bytes, Some(65_536));

        Cli::try_parse_from(["teshi", "browser", "network", "start"])
            .expect_err("network start without --host must fail");
    }

    #[test]
    fn browser_network_start_rejects_non_hostname_filters() {
        for invalid in [
            "https://api.example.test",
            "api.example.test:443",
            "api.example.test/v1",
            "*.example.test",
            "user@example.test",
        ] {
            let result =
                Cli::try_parse_from(["teshi", "browser", "network", "start", "--host", invalid]);
            assert!(result.is_err(), "{invalid} must be rejected");
        }
    }

    #[test]
    fn mcp_stdio_server_accepts_project_root() {
        let cli = Cli::try_parse_from(["teshi", "mcp", "serve", "--stdio", "--project", "."])
            .expect("parse MCP server command");
        let Some(Command::Mcp {
            action: McpCommand::Serve(args),
        }) = cli.command
        else {
            panic!("expected mcp serve subcommand");
        };
        assert!(args.stdio);
        assert_eq!(args.project.as_deref(), Some(std::path::Path::new(".")));
    }

    #[test]
    fn winapp_execute_accepts_uia_selector_and_value_arg() {
        let cli = Cli::try_parse_from([
            "teshi",
            "winapp",
            "execute",
            "--selector",
            "uia:automation_id=SearchBox",
            "--action",
            "fill",
            "--value-arg",
            "hello",
        ])
        .expect("parse winapp execute");
        let Some(Command::WinApp {
            action: WinAppCommand::Execute(args),
        }) = cli.command
        else {
            panic!("expected winapp execute subcommand");
        };
        assert_eq!(args.selector, "uia:automation_id=SearchBox");
        assert_eq!(args.action, "fill");
        assert_eq!(args.value_arg.as_deref(), Some("hello"));
    }

    #[test]
    fn winapp_attach_accepts_title() {
        let cli = Cli::try_parse_from(["teshi", "winapp", "attach", "--title", "My App"])
            .expect("parse winapp attach");
        let Some(Command::WinApp {
            action: WinAppCommand::Attach(args),
        }) = cli.command
        else {
            panic!("expected winapp attach subcommand");
        };
        assert_eq!(args.title.as_deref(), Some("My App"));
    }
}
