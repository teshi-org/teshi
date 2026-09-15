//! ACP Client support for Teshi.
//!
//! This crate intentionally does not depend on any Teshi UI crate or on the
//! native LLM transport. ACP agents are external processes that own their own
//! reasoning/tool loop; Teshi is the client that launches the process,
//! negotiates capabilities, creates sessions, streams updates, answers
//! permission requests, and shuts the process down safely.

pub mod client;
pub mod command;
pub mod error;
pub mod jsonrpc;
pub mod permission;
pub mod registry;

pub use client::{AcpClient, AcpEvent, AcpLifecycle, AcpSession, ClientInfo, InitializeResult};
pub use command::{AcpAgentCommand, CursorAcpConfig, EnvironmentRedaction, resolve_executable};
pub use error::{AcpError, AcpResult};
pub use permission::{PermissionDecision, PermissionHost, PermissionOption, PermissionPolicy};
