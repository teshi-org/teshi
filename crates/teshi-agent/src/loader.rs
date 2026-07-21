//! Agent loader — YAML scanning, path resolution, sub-agent resolution.
//!
//! Three-phase loading pipeline:
//! 1. **Scan** — iterate user and project agent directories for `agent.yaml`.
//! 2. **Resolve paths** — read `system.md`, expand skill/memory paths, expand
//!    `${VAR}` in MCP environment variables.
//! 3. **Resolve sub-agents** — recursively inline sub-agent definitions with
//!    cycle detection and missing-agent tolerance.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::definition::*;

// ─── Public API ──────────────────────────────────────────────────

/// Errors that can occur during agent loading.
#[derive(Debug, Clone)]
pub struct LoadError {
    pub agent_id: String,
    pub kind: LoadErrorKind,
}

#[derive(Debug, Clone)]
pub enum LoadErrorKind {
    /// Filesystem error (missing dir, permission denied, etc.).
    Io { path: PathBuf, message: String },
    /// YAML parse error or semantic error in a specific file.
    Parse { path: PathBuf, detail: String },
    /// Circular `sub_agents` reference.
    CycleDetected {
        /// The full cycle path, e.g. ["default", "reviewer", "default"].
        cycle: Vec<String>,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            LoadErrorKind::Io { path, message } => {
                write!(
                    f,
                    "[{}] I/O error at {}: {}",
                    self.agent_id,
                    path.display(),
                    message
                )
            }
            LoadErrorKind::Parse { path, detail } => {
                write!(
                    f,
                    "[{}] parse error in {}: {}",
                    self.agent_id,
                    path.display(),
                    detail
                )
            }
            LoadErrorKind::CycleDetected { cycle } => {
                write!(
                    f,
                    "[{}] cycle detected in sub_agents: {}",
                    self.agent_id,
                    cycle.join(" → ")
                )
            }
        }
    }
}

#[cfg(test)]
impl std::error::Error for LoadError {}

/// The agent loader — scans, parses, and resolves agent definitions.
pub struct AgentLoader {
    project_dir: Option<PathBuf>,
    #[cfg(test)]
    user_agent_dir_override: Option<PathBuf>,
    #[cfg(test)]
    project_agent_dir_override: Option<PathBuf>,
}

impl AgentLoader {
    /// Create a new loader.
    ///
    /// `project_dir` is the root directory of the project (where `.teshi/`
    /// lives). When `None`, project-tier agents are not loaded.
    pub fn new(project_dir: Option<&Path>) -> Self {
        Self {
            project_dir: project_dir.map(|p| p.to_path_buf()),
            #[cfg(test)]
            user_agent_dir_override: None,
            #[cfg(test)]
            project_agent_dir_override: None,
        }
    }

    #[cfg(test)]
    pub fn with_user_agent_dir(mut self, dir: PathBuf) -> Self {
        self.user_agent_dir_override = Some(dir);
        self
    }

    #[cfg(test)]
    pub fn with_project_agent_dir(mut self, dir: PathBuf) -> Self {
        self.project_agent_dir_override = Some(dir);
        self
    }

    /// Load all agents from all tiers.
    ///
    /// Returns a tuple of (successfully loaded definitions, errors).
    /// Agents with fatal errors (cycle, parse failure) are omitted from
    /// the success list. Non-fatal issues (missing sub-agent) are recorded
    /// in the agent's `missing_sub_agents` field but the agent itself loads.
    pub fn load_all(&self) -> (Vec<AgentDefinition>, Vec<LoadError>) {
        // Phase 1: scan all tiers → raw definitions
        let mut raws: HashMap<String, AgentDefinitionRaw> = HashMap::new();
        let mut dirs: HashMap<String, PathBuf> = HashMap::new();

        // Tier 1: user-level (~/.config/teshi/agents/)
        let user_dir = resolve_user_agent_dir(self);
        if let Some(ref dir) = user_dir {
            scan_tier(dir, &mut raws, &mut dirs);
        }

        // Tier 2: project-level (.teshi/agents/) — overrides user by id
        let project_dir = resolve_project_agent_dir(self);
        if let Some(ref dir) = project_dir {
            scan_tier(dir, &mut raws, &mut dirs);
        }

        // If no agents found on disk, use the built-in minimal fallback.
        // This is the only scenario where built-in code runs.
        if raws.is_empty() {
            return (vec![builtin_minimal()], vec![]);
        }

        // Phase 2 & 3: resolve paths + sub-agents
        let mut resolver = Resolver::new(raws, dirs, self.project_dir.clone());
        resolver.resolve_all()
    }
}

// ─── Tier scanning ──────────────────────────────────────────────

fn user_agent_dir() -> Option<PathBuf> {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    Some(base.join("teshi").join("agents"))
}

/// Resolve the user agent directory, honouring test overrides.
fn resolve_user_agent_dir(_loader: &AgentLoader) -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(ref dir) = _loader.user_agent_dir_override {
        return Some(dir.clone());
    }
    user_agent_dir()
}

/// Resolve the project agent directory, honouring test overrides.
fn resolve_project_agent_dir(loader: &AgentLoader) -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(ref dir) = loader.project_agent_dir_override {
        return Some(dir.clone());
    }
    loader
        .project_dir
        .as_ref()
        .map(|p| p.join(".teshi").join("agents"))
}

/// Scan a single agent directory, inserting into the tiered map.
///
/// Hidden directories (starting with `.`) and directories without
/// an `agent.yaml` file are silently skipped.
fn scan_tier(
    base: &Path,
    raws: &mut HashMap<String, AgentDefinitionRaw>,
    dirs: &mut HashMap<String, PathBuf>,
) {
    let read_dir = match fs::read_dir(base) {
        Ok(d) => d,
        Err(_) => return,
    };

    for entry in read_dir.flatten() {
        let path = entry.path();

        // Skip non-directories and hidden dirs
        if !path.is_dir() {
            continue;
        }
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }

        let yaml_path = path.join("agent.yaml");
        if !yaml_path.exists() {
            continue;
        }

        let content = match fs::read_to_string(&yaml_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let raw: AgentDefinitionRaw = match serde_yaml::from_str(&content) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let id = raw.id.clone();
        // Project tier overrides user tier for same id
        raws.insert(id.clone(), raw);
        dirs.insert(id, path);
    }
}

// ─── Built-in minimal fallback ──────────────────────────────────

/// The absolute last-resort built-in agent.
///
/// Only used when no `agent.yaml` files exist in either the user or
/// project tier. Once a user creates even one agent definition on disk,
/// this fallback is never consulted.
fn builtin_minimal() -> AgentDefinition {
    AgentDefinition {
        id: "default".into(),
        name: "Default".into(),
        description: String::new(),
        system_prompt: "You are a helpful assistant.".into(),
        model_ref: String::new(),
        tools: ToolPermission::All,
        skills: vec![],
        sub_agents: vec![],
        missing_sub_agents: vec![],
        mcp: vec![],
        memory: MemoryConfig { persistent: None },
        compaction: CompactionConfig::default(),
    }
}

// ─── Sub-agent resolver ─────────────────────────────────────────

struct Resolver {
    /// All raw definitions from the scan phase.
    raws: HashMap<String, AgentDefinitionRaw>,
    /// Directory of each agent (for relative-path resolution).
    dirs: HashMap<String, PathBuf>,
    /// Project root for resolving skill directories.
    project_dir: Option<PathBuf>,
    /// Fully resolved definitions cache.
    resolved: HashMap<String, AgentDefinition>,
    /// Current call stack for cycle detection.
    stack: Vec<String>,
    /// Accumulated errors.
    errors: Vec<LoadError>,
}

impl Resolver {
    fn new(
        raws: HashMap<String, AgentDefinitionRaw>,
        dirs: HashMap<String, PathBuf>,
        project_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            raws,
            dirs,
            project_dir,
            resolved: HashMap::new(),
            stack: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Resolve all agents. Returns (loaded agents, errors).
    fn resolve_all(&mut self) -> (Vec<AgentDefinition>, Vec<LoadError>) {
        let ids: Vec<String> = self.raws.keys().cloned().collect();
        for id in &ids {
            if !self.resolved.contains_key(id) {
                self.resolve_one(id);
            }
        }

        self.errors.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));

        let agents: Vec<AgentDefinition> = self.resolved.values().cloned().collect();
        (agents, std::mem::take(&mut self.errors))
    }

    /// Resolve a single agent by id, with memoization and cycle detection.
    fn resolve_one(&mut self, id: &str) -> Option<AgentDefinition> {
        // Memoization: already resolved
        if let Some(def) = self.resolved.get(id) {
            return Some(def.clone());
        }

        // Cycle detection
        if self.stack.contains(&id.to_string()) {
            // Find the start of the cycle in the stack
            let cycle_start = self.stack.iter().position(|x| x == id).unwrap();
            let cycle: Vec<String> = self.stack[cycle_start..].to_vec();
            self.errors.push(LoadError {
                agent_id: id.to_string(),
                kind: LoadErrorKind::CycleDetected { cycle },
            });
            return None;
        }

        // Clone the raw and dir early so the borrow on self.raws/self.dirs
        // is released before any recursive call to resolve_one().
        let raw = self.raws.get(id)?.clone();
        let agent_dir = self.dirs.get(id)?.clone();

        // Push to call stack
        self.stack.push(id.to_string());

        // Resolve sub-agents first (depth-first)
        let mut sub_agents = Vec::new();
        let mut missing_sub_agents = Vec::new();
        if let Some(ref sub_ids) = raw.sub_agents {
            for sub_id in sub_ids {
                match self.resolve_one(sub_id) {
                    Some(def) => sub_agents.push(def),
                    None => missing_sub_agents.push(sub_id.clone()),
                }
            }
        }

        // Pop from call stack
        self.stack.pop();

        // Build the resolved definition
        let def = self.build_definition(&raw, &agent_dir, sub_agents, missing_sub_agents);

        // Cache and return
        self.resolved.insert(id.to_string(), def.clone());
        Some(def)
    }

    /// Construct an `AgentDefinition` from raw + resolved sub-agents.
    fn build_definition(
        &self,
        raw: &AgentDefinitionRaw,
        agent_dir: &Path,
        sub_agents: Vec<AgentDefinition>,
        missing_sub_agents: Vec<String>,
    ) -> AgentDefinition {
        // --- system_prompt ---
        let system_prompt = raw
            .system_prompt
            .as_ref()
            .and_then(|path| {
                let full_path = agent_dir.join(path);
                fs::read_to_string(&full_path).ok()
            })
            .unwrap_or_else(|| "You are a helpful assistant.".into());

        // --- model_ref ---
        let model_ref = raw.model_ref.clone().unwrap_or_default();

        // --- tools ---
        let tools = match &raw.tools {
            None => ToolPermission::All,
            Some(list) if list.is_empty() => ToolPermission::None,
            Some(list) => ToolPermission::Whitelist(list.clone()),
        };

        // --- skills ---
        let skills = self.resolve_skills(raw, agent_dir);

        // --- mcp ---
        let mcp = self.resolve_mcp(raw);

        // --- memory ---
        let memory = self.resolve_memory(raw, agent_dir);

        // --- compaction ---
        let compaction = raw.compaction.clone().unwrap_or_default();

        AgentDefinition {
            id: raw.id.clone(),
            name: raw.name.clone(),
            description: raw.description.clone().unwrap_or_default(),
            system_prompt,
            model_ref,
            tools,
            skills,
            sub_agents,
            missing_sub_agents,
            mcp,
            memory,
            compaction,
        }
    }

    // ── Path resolution helpers ──────────────────────────────

    fn resolve_skills(&self, raw: &AgentDefinitionRaw, agent_dir: &Path) -> Vec<PathBuf> {
        let dirs_to_scan: Vec<String> = match &raw.skills {
            // skills block absent → auto-scan default directories
            None => vec![
                ".teshi/skills".into(),
                // User-level skills dir (convention, same parent as agents)
                "skills".into(),
            ],
            // skills block present
            Some(config) => match &config.dirs {
                // skills: {} or skills: { dirs: ~ } → no skills
                None => return vec![],
                // skills: { dirs: [] } → no skills
                Some(list) if list.is_empty() => return vec![],
                // skills: { dirs: [list] } → use specified dirs
                Some(list) => list.clone(),
            },
        };

        // Resolve each directory relative to project root (or agent dir as fallback)
        let base = self.project_dir.as_deref().unwrap_or(agent_dir);
        dirs_to_scan
            .iter()
            .map(|d| base.join(d))
            .filter(|p| p.is_dir())
            .collect()
    }

    fn resolve_mcp(&self, raw: &AgentDefinitionRaw) -> Vec<McpServerConfig> {
        let Some(ref mcp_raw) = raw.mcp else {
            return vec![];
        };

        mcp_raw
            .servers
            .iter()
            .map(|s| McpServerConfig {
                id: s.id.clone(),
                enabled: s.enabled,
                command: s.command.clone(),
                args: s.args.clone(),
                env: s
                    .env
                    .iter()
                    .map(|(k, v)| (k.clone(), expand_env_var(v)))
                    .collect(),
                timeout_secs: s.timeout_secs,
            })
            .collect()
    }

    fn resolve_memory(&self, raw: &AgentDefinitionRaw, agent_dir: &Path) -> MemoryConfig {
        let Some(ref mem_raw) = raw.memory else {
            return MemoryConfig { persistent: None };
        };

        let persistent = mem_raw.persistent.as_ref().map(|p| match p {
            PersistentMemoryBackendRaw::File { path } => PersistentMemoryBackend::File {
                path: agent_dir.join(path),
            },
        });

        MemoryConfig { persistent }
    }
}

// ─── Environment variable expansion ─────────────────────────────

/// Expand `${VAR_NAME}` patterns in a string using the process environment.
///
/// Supports:
/// - `"${GITHUB_TOKEN}"` → replaced with env var value (or empty string if unset)
/// - `"literal text"` → unchanged
/// - `"prefix_${VAR}_suffix"` → only `${VAR}` portions are expanded
fn expand_env_var(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut rest = value;

    while let Some(dollar) = rest.find("${") {
        // Push everything before the `${`
        result.push_str(&rest[..dollar]);

        // Find the matching `}`
        let after_dollar = &rest[dollar + 2..];
        if let Some(close) = after_dollar.find('}') {
            let var_name = &after_dollar[..close];
            let var_value = std::env::var(var_name).unwrap_or_default();
            result.push_str(&var_value);
            rest = &after_dollar[close + 1..];
        } else {
            // No closing `}` — push the `${` literally and continue
            result.push_str("${");
            rest = after_dollar;
        }
    }

    result.push_str(rest);
    result
}

// ─── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Helper: create a temporary agent.yaml and system.md.
    fn write_agent(dir: &Path, id: &str, system_prompt: Option<&str>) {
        fs::create_dir_all(dir).unwrap();

        let yaml_content = format!(
            r#"id: {id}
name: "Test {id}"
description: "A test agent"
system_prompt: system.md
model_ref: test-model
tools:
  - get_project_info
  - run_tests
skills:
  dirs:
    - skills
compaction:
  enabled: true
  target_ratio: 0.5
  min_keep_chars: 500
"#,
            id = id
        );
        let mut f = fs::File::create(dir.join("agent.yaml")).unwrap();
        f.write_all(yaml_content.as_bytes()).unwrap();

        if let Some(prompt) = system_prompt {
            let mut f = fs::File::create(dir.join("system.md")).unwrap();
            f.write_all(prompt.as_bytes()).unwrap();
        }
    }

    #[test]
    fn load_single_agent() {
        let dir = tempfile::tempdir().unwrap();
        write_agent(
            &dir.path().join("test-agent"),
            "test-agent",
            Some("You are a test agent."),
        );

        let raws = &mut HashMap::new();
        let dirs = &mut HashMap::new();
        scan_tier(dir.path(), raws, dirs);

        assert_eq!(raws.len(), 1);
        assert!(raws.contains_key("test-agent"));
        assert!(dirs.contains_key("test-agent"));
    }

    #[test]
    fn skip_hidden_dirs() {
        let dir = tempfile::tempdir().unwrap();
        // Hidden dir — should be skipped
        write_agent(&dir.path().join(".hidden-agent"), "hidden", Some("p"));
        // Visible dir — should be loaded
        write_agent(&dir.path().join("visible"), "visible", Some("p"));

        let raws = &mut HashMap::new();
        let dirs = &mut HashMap::new();
        scan_tier(dir.path(), raws, dirs);

        assert_eq!(raws.len(), 1);
        assert!(raws.contains_key("visible"));
        assert!(!raws.contains_key("hidden"));
    }

    #[test]
    fn skip_dir_without_agent_yaml() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("no-yaml")).unwrap();

        let raws = &mut HashMap::new();
        let dirs = &mut HashMap::new();
        scan_tier(dir.path(), raws, dirs);

        assert!(raws.is_empty());
    }

    #[test]
    fn load_with_override() {
        let base = tempfile::tempdir().unwrap();

        // User tier
        let user_dir = base.path().join("user_agents");
        write_agent(&user_dir.join("agent-a"), "agent-a", Some("user version"));
        write_agent(&user_dir.join("agent-b"), "agent-b", Some("user version"));

        // Project tier — overrides agent-a
        let project_agent_dir = base.path().join("project_agents");
        write_agent(
            &project_agent_dir.join("agent-a"),
            "agent-a",
            Some("project version"),
        );

        let loader = AgentLoader::new(None)
            .with_user_agent_dir(user_dir)
            .with_project_agent_dir(project_agent_dir);
        let (agents, errors) = loader.load_all();

        assert!(errors.is_empty(), "unexpected errors: {errors:?}");

        let agent_a = agents.iter().find(|a| a.id == "agent-a").unwrap();
        assert_eq!(agent_a.system_prompt, "project version");

        let agent_b = agents.iter().find(|a| a.id == "agent-b").unwrap();
        assert_eq!(agent_b.system_prompt, "user version");
    }

    #[test]
    fn builtin_fallback_when_empty() {
        let loader = AgentLoader::new(None);
        let (agents, errors) = loader.load_all();

        assert!(errors.is_empty());
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "default");
        assert_eq!(agents[0].system_prompt, "You are a helpful assistant.");
        assert!(matches!(agents[0].tools, ToolPermission::All));
    }

    #[test]
    fn no_fallback_when_agents_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let project_agents = dir.path().join(".teshi").join("agents");
        let custom_dir = project_agents.join("custom");
        write_agent(&custom_dir, "custom", Some("custom prompt"));

        let loader = AgentLoader::new(Some(dir.path()));
        let (agents, errors) = loader.load_all();

        assert!(errors.is_empty());
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "custom");
        assert_eq!(agents[0].system_prompt, "custom prompt");
    }

    #[test]
    fn expand_env_var_basic() {
        unsafe { std::env::set_var("TESHI_TEST_VAR", "expanded_value") };
        assert_eq!(expand_env_var("${TESHI_TEST_VAR}"), "expanded_value");
        assert_eq!(expand_env_var("literal"), "literal");
        assert_eq!(
            expand_env_var("prefix_${TESHI_TEST_VAR}_suffix"),
            "prefix_expanded_value_suffix"
        );
    }

    #[test]
    fn expand_env_var_unset() {
        // Unset var should expand to empty string
        let result = expand_env_var("${DOES_NOT_EXIST_XYZ123}");
        assert_eq!(result, "");
    }
}
