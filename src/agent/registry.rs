//! Agent registry — query interface over loaded agent definitions.
//!
//! Created by [`AgentLoader`], consumed by the App. The registry owns
//! the fully-resolved [`AgentDefinition`] values and provides lookup
//! by id, by index, and default-agent access.

use std::path::Path;

use crate::agent::definition::AgentDefinition;
use crate::agent::loader::{AgentLoader, LoadError};

/// Registry of all loaded agent definitions.
///
/// This is the single point of access for the App — it never touches
/// the [`AgentLoader`] or [`AgentDefinitionRaw`] types directly.
#[derive(Debug)]
pub struct AgentRegistry {
    definitions: Vec<AgentDefinition>,
    errors: Vec<LoadError>,
}

impl AgentRegistry {
    /// Load all agents from disk using the default tiered loading.
    ///
    /// `project_dir` is the root of the project (where `.teshi/` lives).
    /// Pass `None` to skip project-tier agents.
    pub fn load(project_dir: Option<&Path>) -> Self {
        let loader = AgentLoader::new(project_dir);
        let (definitions, errors) = loader.load_all();
        Self {
            definitions,
            errors,
        }
    }

    /// Number of loaded agent definitions.
    pub fn len(&self) -> usize {
        self.definitions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }

    /// Get a reference to an agent definition by id.
    pub fn get(&self, id: &str) -> Option<&AgentDefinition> {
        self.definitions.iter().find(|d| d.id == id)
    }

    /// Get a reference to an agent definition by index.
    pub fn get_index(&self, index: usize) -> Option<&AgentDefinition> {
        self.definitions.get(index)
    }

    /// Iterate all loaded agent definitions.
    pub fn iter(&self) -> impl Iterator<Item = &AgentDefinition> {
        self.definitions.iter()
    }

    /// Return the default agent definition.
    ///
    /// The default is the agent with id `"default"`, or the first
    /// loaded definition if none has that id. Panics only when the
    /// registry is empty (which cannot happen in normal operation
    /// because the loader always produces at least the built-in
    /// minimal fallback).
    pub fn default(&self) -> &AgentDefinition {
        self.definitions
            .iter()
            .find(|d| d.id == "default")
            .unwrap_or_else(|| {
                // Safety: the loader always produces at least one agent.
                &self.definitions[0]
            })
    }

    /// Non-fatal loading errors (parse failures, missing sub-agents, etc.).
    pub fn errors(&self) -> &[LoadError] {
        &self.errors
    }
}

impl IntoIterator for AgentRegistry {
    type Item = AgentDefinition;
    type IntoIter = std::vec::IntoIter<AgentDefinition>;

    fn into_iter(self) -> Self::IntoIter {
        self.definitions.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    fn write_agent(dir: &std::path::Path, id: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let yaml = format!(
            r#"id: {id}
name: "Agent {id}"
description: "test"
system_prompt: system.md
model_ref: test-model
"#
        );
        let mut f = std::fs::File::create(dir.join("agent.yaml")).unwrap();
        f.write_all(yaml.as_bytes()).unwrap();

        let mut f = std::fs::File::create(dir.join("system.md")).unwrap();
        f.write_all(b"You are a test agent.").unwrap();
    }

    fn make_project_agent_dir() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempdir().unwrap();
        let agent_dir = dir.path().join(".teshi").join("agents");
        (dir, agent_dir)
    }

    #[test]
    fn load_and_query_by_id() {
        let (_tmp, agent_dir) = make_project_agent_dir();
        write_agent(&agent_dir.join("alpha"), "alpha");
        write_agent(&agent_dir.join("beta"), "beta");

        let reg = AgentRegistry::load(Some(_tmp.path()));
        assert_eq!(reg.len(), 2);

        let alpha = reg.get("alpha").expect("alpha should exist");
        assert_eq!(alpha.name, "Agent alpha");

        let beta = reg.get("beta").expect("beta should exist");
        assert_eq!(beta.name, "Agent beta");

        assert!(reg.get("gamma").is_none());
    }

    #[test]
    fn default_is_first_with_id_default() {
        let (_tmp, agent_dir) = make_project_agent_dir();
        write_agent(&agent_dir.join("default"), "default");
        write_agent(&agent_dir.join("other"), "other");

        let reg = AgentRegistry::load(Some(_tmp.path()));
        assert_eq!(reg.default().id, "default");
    }

    #[test]
    fn default_falls_back_to_first() {
        let (_tmp, agent_dir) = make_project_agent_dir();
        write_agent(&agent_dir.join("alpha"), "alpha");

        let reg = AgentRegistry::load(Some(_tmp.path()));
        assert_eq!(reg.default().id, "alpha");
    }

    #[test]
    fn builtin_fallback_when_no_agents() {
        let reg = AgentRegistry::load(None);
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.default().id, "default");
        assert!(reg.errors().is_empty());
    }

    #[test]
    fn get_index() {
        let (_tmp, agent_dir) = make_project_agent_dir();
        write_agent(&agent_dir.join("a"), "a");
        write_agent(&agent_dir.join("b"), "b");

        let reg = AgentRegistry::load(Some(_tmp.path()));
        // Order depends on filesystem iteration — just verify both exist
        let ids: std::collections::HashSet<&str> =
            reg.iter().map(|d| d.id.as_str()).collect();
        assert!(ids.contains("a"), "agent 'a' should be present");
        assert!(ids.contains("b"), "agent 'b' should be present");
        // Index access should not panic and should return valid entries
        for i in 0..reg.len() {
            assert!(reg.get_index(i).is_some(), "index {i} should have an agent");
        }
        assert!(reg.get_index(reg.len()).is_none(), "past-the-end should be None");
    }

    #[test]
    fn iter_all() {
        let (_tmp, agent_dir) = make_project_agent_dir();
        write_agent(&agent_dir.join("x"), "x");
        write_agent(&agent_dir.join("y"), "y");

        let reg = AgentRegistry::load(Some(_tmp.path()));
        let ids: std::collections::HashSet<&str> =
            reg.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids.len(), 2, "should have exactly 2 agents");
        assert!(ids.contains("x"));
        assert!(ids.contains("y"));
    }

    #[test]
    fn into_iter_consumes() {
        let (_tmp, agent_dir) = make_project_agent_dir();
        write_agent(&agent_dir.join("a"), "a");

        let reg = AgentRegistry::load(Some(_tmp.path()));
        let count = reg.into_iter().count();
        assert_eq!(count, 1);
    }

    #[test]
    fn empty_and_len() {
        let reg = AgentRegistry::load(None);
        assert!(!reg.is_empty());
        assert_eq!(reg.len(), 1);
    }
}
