//! ACP Client support for Teshi.
//!
//! This crate intentionally does not depend on any Teshi UI crate or on the
//! native LLM transport. ACP agents are external processes that own their own
//! reasoning/tool loop; Teshi is the client that launches the process,
//! negotiates capabilities, creates sessions, streams updates, answers
//! permission requests, and shuts the process down safely.
//!
//! Spawned agents retain an owned process container through shutdown. Windows
//! agents start suspended, enter a non-breakaway Job, and resume only after
//! assignment; shutdown waits for that Job to become empty. Unix agents start
//! in a dedicated process group, which is terminated even after the primary
//! agent exits. Explicit Unix session/group escape is outside this mechanism.
//! A failed shutdown remains retryable rather than acknowledging completion.

pub mod client;
pub mod command;
pub mod error;
pub mod jsonrpc;
pub mod permission;
mod process_tree;
pub mod profile;
pub mod registry;
pub mod session_config;

/// Official ACP schema types for client-provided MCP server descriptors.
pub use agent_client_protocol::schema::v1::{EnvVariable, McpServer, McpServerStdio};

pub use client::{AcpClient, AcpEvent, AcpLifecycle, AcpSession, ClientInfo, InitializeResult};
pub use command::{AcpAgentCommand, CursorAcpConfig, EnvironmentRedaction, resolve_executable};
pub use error::{AcpError, AcpResult};
pub use permission::{PermissionDecision, PermissionHost, PermissionOption, PermissionPolicy};
pub use profile::AcpAgentProfile;
