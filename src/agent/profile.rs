//! Agent profile system — configurable agent personas.
//!
//! Each profile defines an agent's system prompt, enabled tool set, optional
//! model binding, and skill directories. Profiles are loaded from TOML files
//! stored in `~/.config/teshi/agents/` (user) and `.teshi/agents/` (project).
//! A built-in default profile is compiled into the binary.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// An agent profile definition.
///
/// Profiles are stored as individual TOML files. The `id` field corresponds
/// to the filename stem and must be unique across all loaded profiles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    /// Unique identifier (filename stem, e.g. `"bdd-writer"`).
    pub id: String,
    /// Human-readable label shown in the selection panel.
    pub name: String,
    /// One-liner description shown below the name.
    #[serde(default)]
    pub description: String,
    /// System prompt / instructions for the agent.
    /// When empty, the built-in default prompt is used.
    #[serde(default)]
    pub instructions: String,
    /// Tool names to enable for this agent.
    /// Empty = all available tools.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Optional reference to a ModelProfile ID to auto-activate on switch.
    #[serde(default)]
    pub model_ref: Option<String>,
    /// Directories (relative to project root) to scan for `.tskill` skill files.
    #[serde(default)]
    pub skills_dirs: Vec<String>,
}

impl AgentProfile {
    /// Build the instructions text, falling back to the built-in default.
    pub fn instructions_or_default(&self) -> String {
        if self.instructions.is_empty() {
            default_builtin_instructions()
        } else {
            self.instructions.clone()
        }
    }

    /// Generate a unique hex ID.
    pub fn generate_id() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let pid = std::process::id();
        format!("{:016x}{:08x}", nanos, pid)
    }

    /// Create a new profile with a fresh unique ID.
    pub fn new(name: &str) -> Self {
        Self {
            id: Self::generate_id(),
            name: name.to_string(),
            description: String::new(),
            instructions: String::new(),
            tools: vec![],
            model_ref: None,
            skills_dirs: vec![],
        }
    }

    /// User-level storage directory for agent profile TOML files.
    pub fn storage_dir() -> std::path::PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("teshi")
            .join("agents")
    }

    /// File path for this profile's TOML.
    pub fn file_path(&self) -> std::path::PathBuf {
        Self::storage_dir().join(format!("{}.toml", self.id))
    }

    /// Save this profile to its TOML file (creates parent dir if needed).
    pub fn save_to_disk(&self) -> Result<(), Box<dyn std::error::Error>> {
        let dir = Self::storage_dir();
        std::fs::create_dir_all(&dir)?;
        std::fs::write(self.file_path(), toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Delete this profile's TOML file from disk.
    pub fn delete_from_disk(&self) -> Result<(), Box<dyn std::error::Error>> {
        let path = self.file_path();
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}

/// Simple registry of agent profiles.
#[derive(Debug)]
pub struct AgentProfileRegistry {
    profiles: Vec<AgentProfile>,
}

impl AgentProfileRegistry {
    /// Load all profiles from all sources, sorted by priority.
    ///
    /// Loading order (later overrides same `id`):
    /// 1. Built-in default
    /// 2. User profiles from `~/.config/teshi/agents/*.toml`
    /// 3. Project profiles from `.teshi/agents/*.toml`
    pub fn load_all(project_dir: Option<&Path>) -> Self {
        let mut profiles: Vec<AgentProfile> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

        // 1. Built-in default — always included first
        let builtin = builtin_default();
        seen.insert(builtin.id.clone());
        profiles.push(builtin);

        // 2. User profiles
        if let Some(user_dir) = user_storage_dir() {
            for p in load_from_dir(&user_dir) {
                if seen.insert(p.id.clone()) {
                    profiles.push(p);
                }
            }
        }

        // 3. Project profiles
        if let Some(dir) = project_dir {
            let project_dir = dir.join(".teshi").join("agents");
            for p in load_from_dir(&project_dir) {
                if seen.insert(p.id.clone()) {
                    profiles.push(p);
                }
            }
        }

        Self { profiles }
    }

    /// Get a profile by id.
    #[allow(dead_code)]
    pub fn get(&self, id: &str) -> Option<&AgentProfile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    /// List all loaded profiles.
    pub fn list(&self) -> &[AgentProfile] {
        &self.profiles
    }

    /// Return the default profile (always the built-in `"default"`).
    #[allow(dead_code)]
    pub fn default(&self) -> &AgentProfile {
        self.profiles
            .iter()
            .find(|p| p.id == "default")
            .expect("default profile should always be present")
    }
}

// ── Loading helpers ──────────────────────────────────────────────

fn load_from_dir(path: &Path) -> Vec<AgentProfile> {
    let mut profiles = Vec::new();
    let dir = match std::fs::read_dir(path) {
        Ok(d) => d,
        Err(_) => return profiles,
    };
    for entry in dir.flatten() {
        let p = entry.path();
        if p.extension().is_some_and(|ext| ext == "toml")
            && let Ok(content) = std::fs::read_to_string(&p)
            && let Ok(profile) = toml::from_str::<AgentProfile>(&content)
        {
            profiles.push(profile);
        }
    }
    profiles
}

fn user_storage_dir() -> Option<std::path::PathBuf> {
    let base = dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    Some(base.join("teshi").join("agents"))
}

// ── Built-in default ─────────────────────────────────────────────

fn builtin_default() -> AgentProfile {
    AgentProfile {
        id: "default".into(),
        name: "Default (BDD Writer)".into(),
        description: "BDD/Gherkin feature writing specialist with all tools".into(),
        instructions: default_builtin_instructions(),
        tools: vec![],
        model_ref: None,
        skills_dirs: vec![".teshi/skills".into(), "skills".into()],
    }
}

/// Returns the built-in default system prompt.
pub fn default_builtin_instructions() -> String {
    include_str!("../default_agent_prompt.md").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_default() {
        let reg = AgentProfileRegistry::load_all(None);
        assert!(reg.get("default").is_some());
        assert_eq!(reg.default().id, "default");
    }

    #[test]
    fn default_profile_has_instructions() {
        let profile = builtin_default();
        assert!(!profile.instructions_or_default().is_empty());
    }

    #[test]
    fn load_from_missing_dir_returns_empty() {
        let profiles = load_from_dir(Path::new("/nonexistent/path"));
        assert!(profiles.is_empty());
    }

    #[test]
    fn profile_tools_empty_means_all() {
        let p = builtin_default();
        assert!(p.tools.is_empty());
    }
}
