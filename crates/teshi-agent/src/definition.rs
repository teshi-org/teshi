//! Agent definition types — YAML schema mirror and resolved runtime form.
//!
//! Two-layer design:
//! - [`AgentDefinitionRaw`] — direct 1:1 mapping from `agent.yaml`, used only
//!   during loading.
//! - [`AgentDefinition`] — fully resolved form with all paths expanded, files
//!   read, sub-agents recursively loaded, and optional defaults filled.

use std::path::PathBuf;

use serde::Deserialize;

// ─── Resolved (runtime) types ─────────────────────────────────────

/// A fully loaded and resolved agent definition.
///
/// All paths have been expanded relative to the agent directory,
/// `system_prompt` has been read from disk, and `sub_agents` have
/// been recursively resolved and inlined.
#[derive(Debug, Clone)]
pub struct AgentDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Full system prompt text, loaded from `system.md` or minimal fallback.
    pub system_prompt: String,
    /// Concrete model reference — resolved from `model_ref` or config default.
    pub model_ref: String,
    pub tools: ToolPermission,
    /// Absolute paths to skill directories.
    pub skills: Vec<PathBuf>,
    /// Recursively resolved sub-agent definitions.
    pub sub_agents: Vec<AgentDefinition>,
    /// Sub-agent IDs that were referenced but not found in any tier.
    ///
    /// Recorded for warning display; the parent agent still loads
    /// without them.
    pub missing_sub_agents: Vec<String>,
    pub mcp: Vec<McpServerConfig>,
    pub memory: MemoryConfig,
    pub compaction: CompactionConfig,
}

/// Three-state tool permission.
#[derive(Debug, Clone)]
pub enum ToolPermission {
    /// `tools` was absent/null in YAML — all built-in tools allowed.
    All,
    /// `tools: []` — no tools allowed (chat-only mode).
    None,
    /// `tools: [list]` — only the listed tools are allowed.
    Whitelist(Vec<String>),
}

/// Resolved MCP server configuration.
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub id: String,
    pub enabled: bool,
    pub command: String,
    pub args: Vec<String>,
    /// Environment variables with `${VAR}` already expanded during loading.
    pub env: std::collections::HashMap<String, String>,
    /// Startup timeout in seconds.
    pub timeout_secs: u64,
}

/// Memory subsystem configuration.
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    pub persistent: Option<PersistentMemoryBackend>,
}

/// Persistent memory storage backend.
#[derive(Debug, Clone)]
pub enum PersistentMemoryBackend {
    File {
        /// Absolute path to the memory context file.
        path: PathBuf,
    },
}

/// Context-window compaction configuration.
///
/// Shared between Raw and Resolved — no path fields to resolve.
#[derive(Debug, Clone, Deserialize)]
pub struct CompactionConfig {
    #[serde(default = "default_compaction_enabled")]
    pub enabled: bool,
    /// Context-window fill ratio that triggers compaction (0.0 – 1.0).
    #[serde(default = "default_compaction_ratio")]
    pub target_ratio: f64,
    /// Minimum characters to retain after compaction.
    #[serde(default = "default_compaction_min_chars")]
    pub min_keep_chars: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            target_ratio: 0.7,
            min_keep_chars: 1000,
        }
    }
}

// ─── Raw (YAML mirror) types ────────────────────────────────────

/// Direct 1:1 mapping from `agent.yaml`.
///
/// This type is **only** used during loading. All consumers in the rest
/// of the codebase use [`AgentDefinition`].
#[derive(Debug, Clone, Deserialize)]
pub struct AgentDefinitionRaw {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Relative path to `system.md` (absent = use minimal fallback).
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub model_ref: Option<String>,
    /// `None` = all tools, `Some([])` = no tools, `Some([...])` = whitelist.
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    #[serde(default)]
    pub skills: Option<SkillsConfigRaw>,
    #[serde(default)]
    pub sub_agents: Option<Vec<String>>,
    #[serde(default)]
    pub mcp: Option<McpConfigRaw>,
    #[serde(default)]
    pub memory: Option<MemoryConfigRaw>,
    #[serde(default)]
    pub compaction: Option<CompactionConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SkillsConfigRaw {
    #[serde(default)]
    pub dirs: Option<Vec<String>>,
    // `include` reserved for future use (keyword-based skill filtering).
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpConfigRaw {
    #[serde(default)]
    pub servers: Vec<McpServerConfigRaw>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfigRaw {
    pub id: String,
    #[serde(default = "return_true")]
    pub enabled: bool,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default = "default_mcp_timeout")]
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemoryConfigRaw {
    #[serde(default)]
    pub persistent: Option<PersistentMemoryBackendRaw>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum PersistentMemoryBackendRaw {
    #[serde(rename = "file")]
    File { path: String },
}

// ─── Default-value helpers for serde ──────────────────────────────

fn return_true() -> bool {
    true
}

fn default_compaction_enabled() -> bool {
    true
}

fn default_compaction_ratio() -> f64 {
    0.7
}

fn default_compaction_min_chars() -> usize {
    1000
}

fn default_mcp_timeout() -> u64 {
    30
}
