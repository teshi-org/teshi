use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSnapshot {
    pub root: PathBuf,
    pub features: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProposedChange {
    WriteFeature {
        path: PathBuf,
        content: String,
    },
    UpdateLocator {
        feature: PathBuf,
        step: String,
        locator: String,
    },
}

pub type ProposalId = String;
pub type RunId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRequest {
    pub feature: PathBuf,
    pub scenario: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserCommand {
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

pub type BrowserResult = Value;

/// Capabilities the agent requires from an application shell.
pub trait AgentHost {
    fn project_snapshot(&self) -> Result<ProjectSnapshot> {
        anyhow::bail!("project snapshots are not supported by this host")
    }
    fn read_feature(&self, _path: &Path) -> Result<String> {
        anyhow::bail!("feature reads are not supported by this host")
    }
    fn propose_change(&mut self, _change: ProposedChange) -> Result<ProposalId> {
        anyhow::bail!("change proposals are not supported by this host")
    }
    fn run_tests(&mut self, _request: RunRequest) -> Result<RunId> {
        anyhow::bail!("test runs are not supported by this host")
    }
    fn browser_command(&mut self, _command: BrowserCommand) -> Result<BrowserResult> {
        anyhow::bail!("browser commands are not supported by this host")
    }

    /// Dispatch a registered agent tool through the shell adapter.
    fn execute_tool(
        &mut self,
        name: &str,
        args_json: &str,
        tool_call_id: &str,
        agent_idx: usize,
    ) -> Result<String>;
}

pub fn execute_tool(
    host: &mut dyn AgentHost,
    name: &str,
    args_json: &str,
    tool_call_id: &str,
    agent_idx: usize,
) -> Result<String> {
    host.execute_tool(name, args_json, tool_call_id, agent_idx)
}
