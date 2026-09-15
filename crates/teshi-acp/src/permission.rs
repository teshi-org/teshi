use serde::{Deserialize, Serialize};

use crate::error::{AcpError, AcpResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionOption {
    #[serde(alias = "optionId")]
    pub id: String,
    #[serde(default, alias = "name")]
    pub label: Option<String>,
    /// Best-effort semantic classification derived from option id/label.
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Selected { option_id: String },
    Rejected,
}

pub trait PermissionHost: Send + Sync {
    fn request_permission<'a>(
        &'a self,
        options: &'a [PermissionOption],
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = AcpResult<PermissionDecision>> + Send + 'a>,
    >;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionPolicy {
    Manual,
    Auto,
    Bypass,
}

impl PermissionPolicy {
    pub fn choose(self, options: &[PermissionOption]) -> AcpResult<PermissionDecision> {
        match self {
            PermissionPolicy::Manual => Err(AcpError::PermissionFailed(
                "manual permission requires a host decision".into(),
            )),
            PermissionPolicy::Auto => choose_allow(options, false),
            PermissionPolicy::Bypass => choose_allow(options, true),
        }
    }
}

fn choose_allow(options: &[PermissionOption], broad: bool) -> AcpResult<PermissionDecision> {
    let mut once = None;
    let mut always = None;
    for o in options {
        let text = format!(
            "{} {} {}",
            o.id,
            o.label.as_deref().unwrap_or(""),
            o.kind.as_deref().unwrap_or("")
        )
        .to_ascii_lowercase();
        if text.contains("reject") || text.contains("deny") {
            continue;
        }
        if text.contains("always") {
            always = Some(o.id.clone());
        }
        if text.contains("once") || text.contains("allow") {
            once.get_or_insert_with(|| o.id.clone());
        }
    }
    let selected = if broad {
        always.or(once)
    } else {
        once.or(always)
    };
    selected
        .map(|option_id| PermissionDecision::Selected { option_id })
        .ok_or_else(|| {
            AcpError::PermissionFailed("agent supplied no allowing permission option".into())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> Vec<PermissionOption> {
        vec![
            PermissionOption {
                id: "allow_once".into(),
                label: None,
                kind: None,
            },
            PermissionOption {
                id: "allow_always".into(),
                label: None,
                kind: None,
            },
            PermissionOption {
                id: "reject".into(),
                label: None,
                kind: None,
            },
        ]
    }

    #[test]
    fn auto_uses_least_permissive_allow_and_bypass_broad() {
        assert_eq!(
            PermissionPolicy::Auto.choose(&opts()).unwrap(),
            PermissionDecision::Selected {
                option_id: "allow_once".into()
            }
        );
        assert_eq!(
            PermissionPolicy::Bypass.choose(&opts()).unwrap(),
            PermissionDecision::Selected {
                option_id: "allow_always".into()
            }
        );
    }

    #[test]
    fn no_allow_is_safe_failure() {
        let reject = [PermissionOption {
            id: "reject".into(),
            label: Some("Deny".into()),
            kind: None,
        }];
        assert!(PermissionPolicy::Auto.choose(&reject).is_err());
    }
}
