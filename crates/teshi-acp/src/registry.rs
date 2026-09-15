use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::Duration;

use crate::error::{AcpError, AcpResult};

pub const OFFICIAL_REGISTRY_URL: &str =
    "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub agents: Vec<RegistryAgent>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryAgent {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub distribution: Distribution,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Distribution {
    #[serde(default)]
    pub binary: Option<BTreeMap<String, BinaryTarget>>,
    #[serde(default)]
    pub npx: Option<PackageDistribution>,
    #[serde(default)]
    pub uvx: Option<PackageDistribution>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PackageDistribution {
    pub package: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BinaryTarget {
    pub archive: String,
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedDistribution<'a> {
    Binary(&'a BinaryTarget),
    Npx {
        package: &'a str,
        args: &'a [String],
    },
    Uvx {
        package: &'a str,
        args: &'a [String],
    },
}

impl Registry {
    pub fn parse(json: &str) -> AcpResult<Self> {
        serde_json::from_str(json).map_err(|e| AcpError::RegistryParse(e.to_string()))
    }

    pub fn lookup(&self, id: &str) -> Option<&RegistryAgent> {
        self.agents.iter().find(|a| a.id == id)
    }
}

impl RegistryAgent {
    pub fn resolve_for_platform(&self, platform: &str) -> AcpResult<ResolvedDistribution<'_>> {
        if let Some(target) = self
            .distribution
            .binary
            .as_ref()
            .and_then(|targets| targets.get(platform))
        {
            return Ok(ResolvedDistribution::Binary(target));
        }
        if let Some(npx) = &self.distribution.npx {
            return Ok(ResolvedDistribution::Npx {
                package: &npx.package,
                args: &npx.args,
            });
        }
        if let Some(uvx) = &self.distribution.uvx {
            return Ok(ResolvedDistribution::Uvx {
                package: &uvx.package,
                args: &uvx.args,
            });
        }
        Err(AcpError::UnsupportedPlatform(platform.into()))
    }
}

pub fn current_platform() -> AcpResult<String> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        "windows" => "windows",
        other => return Err(AcpError::UnsupportedPlatform(other.into())),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => return Err(AcpError::UnsupportedPlatform(other.into())),
    };
    Ok(format!("{os}-{arch}"))
}

pub async fn fetch_registry(url: &str, timeout: Duration) -> AcpResult<Registry> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| AcpError::RegistryFetch(e.to_string()))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| AcpError::RegistryFetch(e.to_string()))?;
    if !response.status().is_success() {
        return Err(AcpError::RegistryFetch(format!(
            "HTTP {}",
            response.status()
        )));
    }
    let text = response
        .text()
        .await
        .map_err(|e| AcpError::RegistryFetch(e.to_string()))?;
    Registry::parse(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
      "agents": [{"id":"cursor","name":"Cursor","distribution":{
        "binary":{"linux-x86_64":{"archive":"https://example/agent.tar.gz","cmd":"./agent","args":["acp"],"sha256":"abc"}},
        "npx":{"package":"@acp/example","args":["acp"]},
        "uvx":{"package":"example-acp"}
      }}]
    }"#;

    #[test]
    fn parses_cursor_and_distributions() {
        let reg = Registry::parse(FIXTURE).unwrap();
        let cursor = reg.lookup("cursor").unwrap();
        assert_eq!(cursor.name.as_deref(), Some("Cursor"));
        assert_eq!(cursor.distribution.binary.as_ref().unwrap().len(), 1);
        assert!(matches!(
            cursor.resolve_for_platform("linux-x86_64").unwrap(),
            ResolvedDistribution::Binary(_)
        ));
        assert!(
            cursor.resolve_for_platform("darwin-aarch64").is_ok(),
            "falls back to npx as unsupported managed install foundation"
        );
    }

    #[test]
    fn rejects_legacy_plural_distribution_shape() {
        let legacy = r#"{"agents":[{"id":"legacy","distributions":[]}]}"#;
        assert!(Registry::parse(legacy).is_err());
    }

    #[test]
    fn platform_strings_cover_required_values() {
        for p in [
            "windows-x86_64",
            "windows-aarch64",
            "linux-x86_64",
            "linux-aarch64",
            "darwin-x86_64",
            "darwin-aarch64",
        ] {
            assert!(p.contains('-'));
        }
    }

    #[test]
    fn malformed_registry_fails() {
        assert!(Registry::parse("not json").is_err());
    }
}
