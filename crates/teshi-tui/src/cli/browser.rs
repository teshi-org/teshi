use std::collections::BTreeMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use serde_json::json;
use teshi_engine::{
    BrowserAction, BrowserAgentError, BrowserAgentErrorCode, BrowserConsoleLevel,
    BrowserElementInput, BrowserFeatureId, BrowserMode, BrowserOperation, BrowserOperations,
    BrowserPrivilegedCapability, BrowserScreenshotFormat, BrowserTarget, BrowserWaitCondition,
    LocatorIntent, PageContextRevision, PlaywrightLocatorCandidate, RuntimeConfig, StepBinding,
    TeshiEngine, default_browser_service_script, default_winapp_service_script,
    ensure_user_chrome_broker, load_project_settings, open_project, read_active_step,
    resolve_step_bindings, send_sidecar_command_with_timeout, start_browser_sidecar,
    stop_browser_sidecar,
};

use super::browser_endpoint::{
    auto_reconnect_enabled, doctor_endpoint, ensure_sidecar_healthy, read_cdp_endpoint,
    reconnect_embedded, resolve_browser_project_root, write_cdp_endpoint_from_rust,
    write_chrome_broker_endpoint,
};
use super::locator_verify::{LocatorVerifyRecord, append_locator_verify, verify_record_json};
use super::replay_screenshots::{
    ReplayScreenshotEntry, artifact_path_from_screenshot_payload, capture_and_save_screenshot,
    iso_now, load_or_create_index, save_index, save_screenshot_from_artifact,
};
use super::{
    BrowserArtifactCleanupArgs, BrowserAuditArgs, BrowserCdpArgs, BrowserCommand,
    BrowserConsoleCommand, BrowserConsoleListArgs, BrowserConsoleStartArgs,
    BrowserContentSettingArgs, BrowserCookiesArgs, BrowserEvidenceArgs, BrowserExecuteArgs,
    BrowserExtensionsArgs, BrowserGrantCommand, BrowserJavascriptArgs, BrowserLeaseCommand,
    BrowserLocatorArgs, BrowserLocatorVerifyArgs, BrowserNavigateArgs, BrowserNetworkCommand,
    BrowserNetworkDetailArgs, BrowserNetworkListArgs, BrowserNetworkStartArgs, BrowserPdfArgs,
    BrowserProfileLabelCommand, BrowserReconnectArgs, BrowserReplayArgs, BrowserScreenshotArgs,
    BrowserSelectorArgs, BrowserServeEmbeddedArgs, BrowserSnapshotArgs, BrowserTabCommand,
    BrowserTargetArgs, BrowserVerifyArgs,
};

/// Handles `teshi browser ...` subcommands.
pub fn handle_browser_command(action: &BrowserCommand) -> Result<()> {
    let cwd = std::env::current_dir().context("resolve current directory")?;
    let project_root = resolve_browser_project_root(&cwd).unwrap_or(cwd);
    match action {
        BrowserCommand::Sessions => {
            ensure_cli_chrome_broker(&project_root)?;
            run_typed_operation(
                &project_root,
                BrowserOperation::ListBrowserSessions,
                Duration::from_secs(15),
            )
        }
        BrowserCommand::Tabs(args) => run_typed_operation(
            &project_root,
            BrowserOperation::ListBrowserTabs {
                extension_instance_id: args.session.clone(),
            },
            Duration::from_secs(15),
        ),
        BrowserCommand::Lookup(args) => run_typed_operation(
            &project_root,
            BrowserOperation::LookupBrowserSessions {
                extension_instance_id: args.session.clone(),
                profile_label: args.label.clone(),
                browser_name: args.browser_name.clone(),
                tab_id: args.tab,
            },
            Duration::from_secs(15),
        ),
        BrowserCommand::ProfileLabel { action } => profile_label(&project_root, action),
        BrowserCommand::Tab { action } => tab_operation(&project_root, action),
        BrowserCommand::Lease { action } => lease(&project_root, action),
        BrowserCommand::Grant { action } => grant(&project_root, action),
        BrowserCommand::Audit(args) => privileged_audit(&project_root, args),
        BrowserCommand::Javascript(args) => privileged_javascript(&project_root, args),
        BrowserCommand::Cdp(args) => privileged_cdp(&project_root, args),
        BrowserCommand::Cookies(args) => privileged_cookies(&project_root, args),
        BrowserCommand::ContentSetting(args) => privileged_content_setting(&project_root, args),
        BrowserCommand::Extensions(args) => privileged_extensions(&project_root, args),
        BrowserCommand::Snapshot(args) => snapshot(&project_root, args),
        BrowserCommand::Navigate(args) => navigate(&project_root, args),
        BrowserCommand::Highlight(args) => highlight(&project_root, args),
        BrowserCommand::ClearHighlight(target) => clear_highlight(&project_root, target),
        BrowserCommand::Execute(args) => execute(&project_root, args),
        BrowserCommand::Replay(args) => replay(&project_root, args),
        BrowserCommand::ServeEmbedded(args) => serve_embedded(args),
        BrowserCommand::Doctor => doctor(&project_root),
        BrowserCommand::Reconnect(args) => reconnect(&project_root, args),
        BrowserCommand::Verify(args) => verify(&project_root, args),
        BrowserCommand::Enhance(args) => enhance(&project_root, args),
        BrowserCommand::HealExecute(args) => heal_execute(&project_root, args),
        BrowserCommand::Locator(args) => locator(&project_root, args),
        BrowserCommand::LocatorVerify(args) => locator_verify(&project_root, args),
        BrowserCommand::Evidence(args) => evidence(&project_root, args),
        BrowserCommand::Screenshot(args) => screenshot(&project_root, args),
        BrowserCommand::Pdf(args) => pdf(&project_root, args),
        BrowserCommand::Console { action } => console(&project_root, action),
        BrowserCommand::Network { action } => network(&project_root, action),
        BrowserCommand::ArtifactCleanup(args) => artifact_cleanup(&project_root, args),
    }
}

fn grant(project_root: &Path, action: &BrowserGrantCommand) -> Result<()> {
    let operation = match action {
        BrowserGrantCommand::Create(args) => {
            if !args.yes && !args.non_interactive {
                return Err(anyhow!(
                    "privileged grant requires --yes, or --non-interactive with an exact --acknowledge-capability and policy allowlist"
                ));
            }
            let (target, lease_token) = required_target(&args.target)?;
            let capability: BrowserPrivilegedCapability =
                serde_json::from_value(json!(args.capability))?;
            let acknowledged_capability = args
                .acknowledge_capability
                .as_ref()
                .map(|value| serde_json::from_value(json!(value)))
                .transpose()?;
            BrowserOperation::CreateBrowserCapabilityGrant {
                target,
                lease_token,
                capability,
                ttl_secs: args.ttl,
                interactive_confirmed: args.yes,
                non_interactive: args.non_interactive,
                acknowledged_capability,
            }
        }
        BrowserGrantCommand::List(args) => BrowserOperation::ListBrowserCapabilityGrants {
            extension_instance_id: args.session.clone(),
        },
        BrowserGrantCommand::Revoke(args) => BrowserOperation::RevokeBrowserCapabilityGrant {
            grant_id: args.grant_id.clone(),
        },
        BrowserGrantCommand::Expire => BrowserOperation::ExpireBrowserCapabilityGrants,
    };
    run_typed_operation(project_root, operation, Duration::from_secs(15))
}

fn privileged_audit(project_root: &Path, args: &BrowserAuditArgs) -> Result<()> {
    run_typed_operation(
        project_root,
        BrowserOperation::ListBrowserPrivilegedAudit { limit: args.limit },
        Duration::from_secs(15),
    )
}

fn read_project_privileged_input(
    project_root: &Path,
    path: &Path,
    max_bytes: u64,
) -> Result<String> {
    let root = project_root
        .canonicalize()
        .context("canonicalize project root")?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let canonical = candidate
        .canonicalize()
        .context("privileged input file is unavailable")?;
    if !canonical.starts_with(&root) {
        return Err(anyhow!("privileged input file is outside the project root"));
    }
    if canonical.metadata()?.len() > max_bytes {
        return Err(anyhow!(
            "privileged input file exceeds the configured byte limit"
        ));
    }
    fs::read_to_string(canonical).context("read privileged input file")
}

fn privileged_javascript(project_root: &Path, args: &BrowserJavascriptArgs) -> Result<()> {
    let (target, lease_token) = required_target(&args.target)?;
    let (expression, source_kind) = if let Some(expression) = &args.expression {
        (expression.clone(), "inline".to_string())
    } else {
        (
            read_project_privileged_input(
                project_root,
                args.file.as_deref().context("--file is required")?,
                1_048_576,
            )?,
            "file".to_string(),
        )
    };
    run_typed_operation(
        project_root,
        BrowserOperation::ExecutePrivilegedJavascript {
            target,
            lease_token,
            capability_grant_token: args.grant_token.clone(),
            expression,
            source_kind,
            page_context_revision: args.page_revision.clone().map(PageContextRevision),
            timeout_ms: args.timeout_ms,
            max_result_bytes: args.max_result_bytes,
        },
        command_timeout_for_ms(args.timeout_ms),
    )
}

fn privileged_cdp(project_root: &Path, args: &BrowserCdpArgs) -> Result<()> {
    let (target, lease_token) = required_target(&args.target)?;
    let params_text = if let Some(value) = &args.params_json {
        Some(value.clone())
    } else if let Some(path) = &args.params_file {
        Some(read_project_privileged_input(project_root, path, 262_144)?)
    } else {
        None
    };
    let params = params_text
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .context("CDP params must be a JSON object")?
        .unwrap_or_else(|| json!({}));
    if !params.is_object() {
        return Err(anyhow!("CDP params must be a JSON object"));
    }
    run_typed_operation(
        project_root,
        BrowserOperation::ExecutePrivilegedCdp {
            target,
            lease_token,
            capability_grant_token: args.grant_token.clone(),
            method: args.method.clone(),
            params,
            page_context_revision: args.page_revision.clone().map(PageContextRevision),
            max_result_bytes: args.max_result_bytes,
        },
        Duration::from_secs(30),
    )
}

fn privileged_cookies(project_root: &Path, args: &BrowserCookiesArgs) -> Result<()> {
    let (target, lease_token) = required_target(&args.target)?;
    run_typed_operation(
        project_root,
        BrowserOperation::ListBrowserCookies {
            target,
            lease_token,
            capability_grant_token: args.grant_token.clone(),
            value_capability_grant_token: args.value_grant_token.clone(),
            include_values: args.include_values,
            max_entries: args.max_entries,
            max_result_bytes: args.max_result_bytes,
        },
        Duration::from_secs(30),
    )
}

fn privileged_content_setting(project_root: &Path, args: &BrowserContentSettingArgs) -> Result<()> {
    let (target, lease_token) = required_target(&args.target)?;
    run_typed_operation(
        project_root,
        BrowserOperation::AccessBrowserContentSetting {
            target,
            lease_token,
            capability_grant_token: args.grant_token.clone(),
            setting: args.setting.clone(),
            value: args.value.clone(),
        },
        Duration::from_secs(30),
    )
}

fn privileged_extensions(project_root: &Path, args: &BrowserExtensionsArgs) -> Result<()> {
    let (target, lease_token) = required_target(&args.target)?;
    run_typed_operation(
        project_root,
        BrowserOperation::ListBrowserExtensions {
            target,
            lease_token,
            capability_grant_token: args.grant_token.clone(),
            max_entries: args.max_entries,
        },
        Duration::from_secs(30),
    )
}

fn profile_label(project_root: &Path, action: &BrowserProfileLabelCommand) -> Result<()> {
    let operation = match action {
        BrowserProfileLabelCommand::Set(args) => BrowserOperation::SetBrowserProfileLabel {
            extension_instance_id: args.session.clone(),
            profile_label: args.label.clone(),
        },
        BrowserProfileLabelCommand::Clear(args) => BrowserOperation::ClearBrowserProfileLabel {
            extension_instance_id: args.session.clone(),
        },
    };
    run_typed_operation(project_root, operation, Duration::from_secs(15))
}

fn tab_operation(project_root: &Path, action: &BrowserTabCommand) -> Result<()> {
    let operation = match action {
        BrowserTabCommand::Open(args) => {
            let (target, lease_token) = required_target(&args.target)?;
            BrowserOperation::OpenBrowserTab {
                target,
                lease_token,
                url: args.url.clone(),
                active: args.active,
            }
        }
        BrowserTabCommand::Close(args) => {
            let (target, lease_token) = required_target(args)?;
            BrowserOperation::CloseBrowserTab {
                target,
                lease_token,
            }
        }
        BrowserTabCommand::Activate(args) => {
            let (target, lease_token) = required_target(&args.target)?;
            BrowserOperation::ActivateBrowserTab {
                target,
                lease_token,
                focus_window: args.focus_window,
            }
        }
        BrowserTabCommand::NewWindow(args) => {
            let (target, lease_token) = required_target(&args.target)?;
            BrowserOperation::CreateBrowserWindow {
                target,
                lease_token,
                url: args.url.clone(),
                focused: args.focused,
            }
        }
        BrowserTabCommand::Group(args) => {
            let (target, lease_token) = required_target(&args.target)?;
            BrowserOperation::GroupBrowserTabs {
                target,
                lease_token,
                tab_ids: args.tab_ids.clone(),
                title: args.title.clone(),
            }
        }
    };
    run_typed_operation(project_root, operation, Duration::from_secs(30))
}

fn ensure_cli_chrome_broker(project_root: &Path) -> Result<()> {
    let endpoint = ensure_user_chrome_broker(project_root, &default_browser_service_script())
        .map_err(|error| match error.hint {
            Some(hint) => anyhow!("{} ({hint})", error.message),
            None => anyhow!(error.message),
        })?;
    write_chrome_broker_endpoint(project_root, &endpoint)
}

fn lease(project_root: &Path, action: &BrowserLeaseCommand) -> Result<()> {
    let operation = match action {
        BrowserLeaseCommand::Acquire(args) => BrowserOperation::AcquireBrowserLease {
            extension_instance_id: args.session.clone(),
            owner_label: args.owner.clone(),
            ttl_secs: args.ttl,
        },
        BrowserLeaseCommand::Renew(args) => BrowserOperation::RenewBrowserLease {
            extension_instance_id: args.session.clone(),
            lease_token: args.lease_token.clone(),
            ttl_secs: args.ttl,
        },
        BrowserLeaseCommand::Release(args) => BrowserOperation::ReleaseBrowserLease {
            extension_instance_id: args.session.clone(),
            lease_token: args.lease_token.clone(),
        },
    };
    run_typed_operation(project_root, operation, Duration::from_secs(15))
}

fn locator(project_root: &Path, args: &BrowserLocatorArgs) -> Result<()> {
    let operation = locator_operation(project_root, args)?;
    run_typed_operation(
        project_root,
        operation,
        Duration::from_millis(args.timeout_ms),
    )
}

pub(crate) fn locator_operation(
    project_root: &Path,
    args: &BrowserLocatorArgs,
) -> Result<BrowserOperation> {
    let (target, lease_token) = required_target(&args.target)?;
    let configured = if args.test_id_attributes.is_empty() {
        load_project_settings(project_root)?.playwright_test_id_attributes
    } else {
        args.test_id_attributes.clone()
    };
    Ok(BrowserOperation::ResolvePlaywrightLocator {
        target,
        lease_token,
        intent: LocatorIntent {
            purpose: args.purpose.clone(),
            text: args.text.clone(),
            role: args.role.clone(),
            element_ref: args.element_ref.clone(),
            gherkin_step: args.gherkin_step.clone(),
        },
        test_id_attributes: configured,
    })
}

fn locator_verify(project_root: &Path, args: &BrowserLocatorVerifyArgs) -> Result<()> {
    let (target, lease_token) = required_target(&args.target)?;
    let candidate: PlaywrightLocatorCandidate = serde_json::from_str(&args.candidate_json)
        .context("parse --candidate-json as a Playwright locator candidate")?;
    run_typed_operation(
        project_root,
        BrowserOperation::VerifyPlaywrightLocator {
            target,
            lease_token,
            candidate,
            page_context_revision: PageContextRevision(args.page_revision.clone()),
        },
        Duration::from_millis(args.timeout_ms),
    )
}

fn evidence(project_root: &Path, args: &BrowserEvidenceArgs) -> Result<()> {
    let (target, lease_token) = required_target(&args.target)?;
    run_typed_operation(
        project_root,
        BrowserOperation::CaptureBrowserEvidence {
            target,
            lease_token,
            page_context_revision: PageContextRevision(args.page_revision.clone()),
        },
        Duration::from_millis(args.timeout_ms),
    )
}

fn screenshot(project_root: &Path, args: &BrowserScreenshotArgs) -> Result<()> {
    let (target, lease_token) = required_target(&args.target)?;
    let format = match args.format.to_ascii_lowercase().as_str() {
        "png" => BrowserScreenshotFormat::Png,
        "jpg" | "jpeg" => BrowserScreenshotFormat::Jpeg,
        other => {
            return Err(anyhow!(
                "unsupported screenshot format {other}; use png or jpeg"
            ));
        }
    };
    if matches!(format, BrowserScreenshotFormat::Png) && args.quality.is_some() {
        return Err(anyhow!("--quality is supported only with --format jpeg"));
    }
    if args.quality.is_some_and(|quality| quality > 100) {
        return Err(anyhow!("--quality must be between 0 and 100"));
    }
    let candidate = args
        .candidate_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .context("parse --candidate-json")?;
    let element = BrowserElementInput {
        reference: args.reference.clone(),
        candidate,
        css: args.selector.clone(),
        snapshot_id: args.snapshot_id.clone(),
        page_context_revision: args.page_revision.clone().map(PageContextRevision),
    };
    let element =
        if element.reference.is_some() || element.candidate.is_some() || element.css.is_some() {
            element.validate().map_err(|message| anyhow!(message))?;
            if args.full_page {
                return Err(anyhow!(
                    "--full-page cannot be combined with an element target"
                ));
            }
            Some(Box::new(element))
        } else {
            None
        };
    run_typed_operation(
        project_root,
        BrowserOperation::CaptureBrowserScreenshot {
            target,
            lease_token,
            page_context_revision: args.page_revision.clone().map(PageContextRevision),
            format,
            quality: args.quality,
            full_page: args.full_page,
            element,
        },
        Duration::from_millis(args.timeout_ms),
    )
}

fn pdf(project_root: &Path, args: &BrowserPdfArgs) -> Result<()> {
    let (target, lease_token) = required_target(&args.target)?;
    if !(0.1..=2.0).contains(&args.scale) {
        return Err(anyhow!("--scale must be between 0.1 and 2.0"));
    }
    run_typed_operation(
        project_root,
        BrowserOperation::GenerateBrowserPdf {
            target,
            lease_token,
            page_context_revision: args.page_revision.clone().map(PageContextRevision),
            paper_format: args.paper.clone(),
            landscape: args.landscape,
            scale: args.scale,
            print_background: args.print_background,
        },
        Duration::from_millis(args.timeout_ms),
    )
}

fn artifact_cleanup(project_root: &Path, args: &BrowserArtifactCleanupArgs) -> Result<()> {
    run_typed_operation(
        project_root,
        BrowserOperation::CleanupBrowserArtifacts {
            paths: args
                .paths
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
        },
        Duration::from_secs(15),
    )
}

fn console(project_root: &Path, action: &BrowserConsoleCommand) -> Result<()> {
    let operation = match action {
        BrowserConsoleCommand::Start(args) => console_start_operation(args)?,
        BrowserConsoleCommand::List(args) => console_list_operation(args)?,
        BrowserConsoleCommand::Clear(args) => {
            let (target, lease_token) = required_target(args)?;
            BrowserOperation::ClearBrowserConsoleCapture {
                target,
                lease_token,
            }
        }
        BrowserConsoleCommand::Stop(args) => {
            let (target, lease_token) = required_target(args)?;
            BrowserOperation::StopBrowserConsoleCapture {
                target,
                lease_token,
            }
        }
    };
    run_typed_operation(project_root, operation, Duration::from_secs(30))
}

fn console_start_operation(args: &BrowserConsoleStartArgs) -> Result<BrowserOperation> {
    let (target, lease_token) = required_target(&args.target)?;
    Ok(BrowserOperation::StartBrowserConsoleCapture {
        target,
        lease_token,
        levels: parse_console_levels(&args.level)?,
        max_age_ms: args.max_age_ms,
        max_entries: args.max_entries,
        max_bytes: args.max_bytes,
        sensitive_fields: args.sensitive_fields.clone(),
    })
}

fn console_list_operation(args: &BrowserConsoleListArgs) -> Result<BrowserOperation> {
    let (target, lease_token) = required_target(&args.target)?;
    Ok(BrowserOperation::ListBrowserConsoleEvents {
        target,
        lease_token,
        levels: if args.level.is_empty() {
            None
        } else {
            Some(parse_console_levels(&args.level)?)
        },
        max_age_ms: args.max_age_ms,
        max_entries: args.max_entries,
        max_bytes: args.max_bytes,
    })
}

fn parse_console_levels(values: &[String]) -> Result<Vec<BrowserConsoleLevel>> {
    values
        .iter()
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "debug" => Ok(BrowserConsoleLevel::Debug),
            "log" => Ok(BrowserConsoleLevel::Log),
            "info" => Ok(BrowserConsoleLevel::Info),
            "warn" | "warning" => Ok(BrowserConsoleLevel::Warn),
            "error" => Ok(BrowserConsoleLevel::Error),
            other => Err(anyhow!(
                "unsupported console level {other}; use debug, log, info, warn, or error"
            )),
        })
        .collect()
}

fn network(project_root: &Path, action: &BrowserNetworkCommand) -> Result<()> {
    let operation = match action {
        BrowserNetworkCommand::Start(args) => {
            let operation = network_start_operation(args)?;
            let session = args
                .target
                .session
                .as_deref()
                .ok_or_else(|| anyhow!("--session is required for this browser operation"))?;
            ensure_filtered_network_capture_compatible(project_root, session)?;
            operation
        }
        BrowserNetworkCommand::List(args) => network_list_operation(args)?,
        BrowserNetworkCommand::Detail(args) => network_detail_operation(args)?,
        BrowserNetworkCommand::Clear(args) => {
            let (target, lease_token) = required_target(args)?;
            BrowserOperation::ClearBrowserNetworkCapture {
                target,
                lease_token,
            }
        }
        BrowserNetworkCommand::Stop(args) => {
            let (target, lease_token) = required_target(args)?;
            BrowserOperation::StopBrowserNetworkCapture {
                target,
                lease_token,
            }
        }
    };
    run_typed_operation(project_root, operation, Duration::from_secs(30))
}

fn network_start_operation(args: &BrowserNetworkStartArgs) -> Result<BrowserOperation> {
    let (target, lease_token) = required_target(&args.target)?;
    let allowed_hostnames = args
        .hosts
        .iter()
        .map(|hostname| super::normalize_exact_hostname(hostname))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(anyhow::Error::msg)?;
    Ok(BrowserOperation::StartBrowserNetworkCapture {
        target,
        lease_token,
        allowed_hostnames,
        capture_request_bodies: args.request_body,
        max_request_body_bytes: args.max_request_body_bytes,
        max_age_ms: args.max_age_ms,
        max_entries: args.max_entries,
        max_bytes: args.max_bytes,
        max_body_bytes: args.max_body_bytes,
        sensitive_fields: args.sensitive_fields.clone(),
    })
}

fn ensure_filtered_network_capture_compatible(project_root: &Path, session: &str) -> Result<()> {
    let discovery = execute_typed_operation_value(
        project_root,
        BrowserOperation::ListBrowserSessions,
        Duration::from_secs(15),
    )?;
    if let Err(error) = validate_filtered_network_capture_capabilities(&discovery, session) {
        eprintln!("{}", serde_json::to_string_pretty(&error.to_wire_value())?);
        return Err(error.into());
    }
    Ok(())
}

fn validate_filtered_network_capture_capabilities(
    discovery: &serde_json::Value,
    session: &str,
) -> std::result::Result<(), BrowserAgentError> {
    let selected = discovery
        .get("sessions")
        .and_then(serde_json::Value::as_array)
        .and_then(|sessions| {
            sessions.iter().find(|candidate| {
                candidate
                    .pointer("/identity/extension_instance_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(session)
            })
        })
        .ok_or_else(|| BrowserAgentError {
            code: BrowserAgentErrorCode::BrowserTargetNotFound,
            message: "selected browser session is absent from capability discovery".into(),
            recovery: BTreeMap::from([("extension_instance_id".into(), json!(session))]),
        })?;

    let required = [
        BrowserFeatureId::P1_FILTERED_NETWORK_CAPTURE,
        BrowserFeatureId::P1_NETWORK_BATCH_TRANSPORT,
    ];
    let features = selected
        .pointer("/capabilities/features")
        .and_then(serde_json::Value::as_array);
    let missing = required
        .iter()
        .filter(|required_feature| {
            !features.is_some_and(|items| {
                items.iter().any(|item| {
                    item.get("feature").and_then(serde_json::Value::as_str)
                        == Some(**required_feature)
                        && item.get("available").and_then(serde_json::Value::as_bool) == Some(true)
                })
            })
        })
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }

    Err(BrowserAgentError {
        code: BrowserAgentErrorCode::BrowserCapabilityUnavailable,
        message: "selected browser session cannot safely start filtered network capture".into(),
        recovery: BTreeMap::from([
            ("extension_instance_id".into(), json!(session)),
            ("required_features".into(), json!(required)),
            ("missing_features".into(), json!(missing)),
        ]),
    })
}

fn network_list_operation(args: &BrowserNetworkListArgs) -> Result<BrowserOperation> {
    let (target, lease_token) = required_target(&args.target)?;
    Ok(BrowserOperation::ListBrowserNetworkRequests {
        target,
        lease_token,
        max_age_ms: args.max_age_ms,
        max_entries: args.max_entries,
        max_bytes: args.max_bytes,
    })
}

fn network_detail_operation(args: &BrowserNetworkDetailArgs) -> Result<BrowserOperation> {
    let (target, lease_token) = required_target(&args.target)?;
    Ok(BrowserOperation::GetBrowserNetworkRequestDetail {
        target,
        lease_token,
        network_request_id: args.network_request_id.clone(),
        include_body: args.include_body,
        max_body_bytes: args.max_body_bytes,
    })
}

fn run_typed_operation(
    project_root: &Path,
    operation: BrowserOperation,
    timeout: Duration,
) -> Result<()> {
    let payload = execute_typed_operation_value(project_root, operation, timeout)?;
    print_json_response(payload)
}

fn execute_typed_operation_value(
    project_root: &Path,
    operation: BrowserOperation,
    timeout: Duration,
) -> Result<serde_json::Value> {
    let endpoint = read_cdp_endpoint(project_root)?;
    let client = BrowserOperations::new(endpoint.ws_url, timeout)
        .with_caller_label("teshi-cli")
        .with_project_root(project_root.to_string_lossy());
    match client.execute(&operation) {
        Ok(response) => Ok(response.payload),
        Err(error) => {
            eprintln!("{}", serde_json::to_string_pretty(&error.to_wire_value())?);
            Err(error.into())
        }
    }
}

fn doctor(project_root: &Path) -> Result<()> {
    let report = doctor_endpoint(project_root)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !report.ok {
        std::process::exit(1);
    }
    Ok(())
}

fn reconnect(project_root: &Path, args: &BrowserReconnectArgs) -> Result<()> {
    let endpoint = reconnect_embedded(project_root, args.navigate.as_deref(), args.wait_secs)?;
    let report = doctor_endpoint(project_root)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "ok": report.ok,
            "ws_url": endpoint.ws_url,
            "mode": endpoint.mode,
            "page_url": endpoint.page_url,
            "doctor": report
        }))?
    );
    if !report.ok {
        std::process::exit(1);
    }
    Ok(())
}

fn verify(project_root: &Path, args: &BrowserVerifyArgs) -> Result<()> {
    let timeout = command_timeout_for_ms(args.timeout_ms);
    let highlight_command = apply_targeting(
        json!({
            "cmd": "highlight_selector",
            "request_id": "browser-verify-highlight",
            "selector": args.selector
        }),
        &args.target,
    )?;
    let highlight = send_browser_command(
        project_root,
        highlight_command,
        Duration::from_secs(20),
        false,
    )?;
    let highlight_ok = highlight.get("ok").and_then(|v| v.as_bool()) == Some(true);
    let response = if args.action == "open_project" {
        open_project_via_sidecar(
            project_root,
            args.value_arg.as_deref().unwrap_or(&args.selector),
            timeout,
            "browser-verify-open-project",
            Some(&args.target),
        )?
    } else if args.action == "navigate" {
        let url = args
            .value_arg
            .as_deref()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or(&args.selector);
        if args.target.session.is_some() {
            let (target, lease_token) = required_target(&args.target)?;
            execute_typed_operation_value(
                project_root,
                BrowserOperation::NavigateBrowser {
                    target,
                    lease_token,
                    url: url.to_string(),
                    timeout_ms: args.timeout_ms,
                    wait: Some(BrowserWaitCondition::LoadComplete),
                    monitor: false,
                },
                timeout,
            )?
        } else {
            navigate_to_url(
                project_root,
                url,
                args.timeout_ms,
                timeout,
                "browser-verify-navigate",
                false,
                Some(&args.target),
            )?
        }
    } else if args.target.session.is_some() {
        let (target, lease_token) = required_target(&args.target)?;
        let action: BrowserAction = serde_json::from_value(json!(args.action))?;
        execute_typed_operation_value(
            project_root,
            BrowserOperation::ExecuteBrowserAction {
                target,
                lease_token,
                action,
                element: BrowserElementInput {
                    css: Some(args.selector.clone()),
                    ..Default::default()
                },
                value: args.value_arg.clone(),
                files: vec![],
                wait: None,
                timeout_ms: args.timeout_ms,
                focus: false,
                monitor: false,
            },
            timeout,
        )?
    } else {
        execute_locator(
            project_root,
            ExecuteLocatorParams {
                selector: &args.selector,
                action: &args.action,
                value: args.value_arg.as_deref(),
                timeout_ms: args.timeout_ms,
                request_id: "browser-verify-execute",
                health_check: false,
                target: Some(&args.target),
            },
            timeout,
        )?
    };
    let execute_ok = response.get("ok").and_then(|v| v.as_bool()) == Some(true);
    let ok = highlight_ok && execute_ok;
    let record = LocatorVerifyRecord {
        ts_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default(),
        step_line: args.step_line,
        selector: args.selector.clone(),
        action: args.action.clone(),
        value_arg: args.value_arg.clone(),
        ok,
    };
    if ok {
        append_locator_verify(project_root, &record)?;
    }
    let mut output = verify_record_json(project_root, &record);
    if let Some(obj) = output.as_object_mut() {
        obj.insert("highlight_ok".into(), json!(highlight_ok));
        obj.insert("execute_ok".into(), json!(execute_ok));
        obj.insert("response".into(), response);
    }
    println!("{}", serde_json::to_string_pretty(&output)?);
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}

fn navigate(project_root: &Path, args: &BrowserNavigateArgs) -> Result<()> {
    if args.target.session.is_some() {
        let (target, lease_token) = required_target(&args.target)?;
        return run_typed_operation(
            project_root,
            BrowserOperation::NavigateBrowser {
                target,
                lease_token,
                url: args.url.clone(),
                timeout_ms: args.timeout_ms,
                wait: Some(BrowserWaitCondition::LoadComplete),
                monitor: args.monitor,
            },
            command_timeout_for_ms(args.timeout_ms),
        );
    }
    let timeout = command_timeout_for_ms(args.timeout_ms);
    let response = navigate_to_url(
        project_root,
        &args.url,
        args.timeout_ms,
        timeout,
        "browser-navigate",
        true,
        Some(&args.target),
    )?;
    ensure_ok(&response)?;
    print_json_response(response)
}

fn snapshot(project_root: &Path, args: &BrowserSnapshotArgs) -> Result<()> {
    let timeout = Duration::from_millis(args.timeout_ms);
    if args.target.session.is_some() {
        let (target, lease_token) = required_target(&args.target)?;
        return run_typed_operation(
            project_root,
            BrowserOperation::GetPageSnapshot {
                target,
                lease_token,
            },
            timeout,
        );
    }
    let response = send_browser_command(
        project_root,
        json!({ "cmd": "get_page_snapshot", "request_id": "browser-snapshot" }),
        timeout,
        true,
    )?;
    print_json_response(response)
}

fn highlight(project_root: &Path, args: &BrowserSelectorArgs) -> Result<()> {
    let command = apply_targeting(
        json!({
            "cmd": "highlight_selector",
            "request_id": "browser-highlight",
            "selector": args.selector
        }),
        &args.target,
    )?;
    let response = send_browser_command(project_root, command, Duration::from_secs(20), false)?;
    print_json_response(response)
}

fn clear_highlight(project_root: &Path, target: &BrowserTargetArgs) -> Result<()> {
    let command = apply_targeting(
        json!({ "cmd": "clear_highlight", "request_id": "browser-clear-highlight" }),
        target,
    )?;
    let response = send_browser_command(project_root, command, Duration::from_secs(10), false)?;
    print_json_response(response)
}

fn execute(project_root: &Path, args: &BrowserExecuteArgs) -> Result<()> {
    let (target, lease_token) = required_target(&args.target)?;
    let action: BrowserAction = serde_json::from_value(json!(args.action)).with_context(|| {
        format!(
            "invalid --action {}; expected click, pointer_click, fill, type, select, press_key, assert_visible, assert_text, navigate, or upload",
            args.action
        )
    })?;
    let candidate = args
        .candidate_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .context("parse --candidate-json")?;
    let element = BrowserElementInput {
        reference: args.reference.clone(),
        candidate,
        css: args.selector.clone(),
        snapshot_id: args.snapshot_id.clone(),
        page_context_revision: args.page_revision.clone().map(PageContextRevision),
    };
    element.validate().map_err(|message| anyhow!(message))?;
    let wait = browser_wait_condition(args, &element)?;
    run_typed_operation(
        project_root,
        BrowserOperation::ExecuteBrowserAction {
            target,
            lease_token,
            action,
            element,
            value: args.value_arg.clone(),
            files: args
                .files
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
            wait: wait.map(Box::new),
            timeout_ms: args.timeout_ms,
            focus: args.focus,
            monitor: args.monitor,
        },
        command_timeout_for_ms(args.timeout_ms),
    )
}

fn browser_wait_condition(
    args: &BrowserExecuteArgs,
    element: &BrowserElementInput,
) -> Result<Option<BrowserWaitCondition>> {
    let mut waits = Vec::new();
    if let Some(pattern) = &args.wait_url {
        waits.push(BrowserWaitCondition::Url {
            pattern: pattern.clone(),
        });
    }
    if let Some(text) = &args.wait_text {
        waits.push(BrowserWaitCondition::VisibleText { text: text.clone() });
    }
    if let Some(state) = &args.wait_state {
        let state = serde_json::from_value(json!(state)).with_context(|| {
            format!("invalid --wait-state {state}; expected visible, hidden, enabled, or disabled")
        })?;
        waits.push(BrowserWaitCondition::ElementState {
            element: Box::new(element.clone()),
            state,
        });
    }
    if args.wait_revision_change {
        let from = args
            .page_revision
            .clone()
            .ok_or_else(|| anyhow!("--wait-revision-change requires --page-revision"))?;
        waits.push(BrowserWaitCondition::PageRevisionChange {
            from: PageContextRevision(from),
        });
    }
    if args.wait_load {
        waits.push(BrowserWaitCondition::LoadComplete);
    }
    if waits.len() > 1 {
        anyhow::bail!("choose only one typed --wait-* condition");
    }
    Ok(waits.pop())
}

fn enhance(project_root: &Path, args: &BrowserSelectorArgs) -> Result<()> {
    let timeout = command_timeout_for_ms(10_000);
    let command = apply_targeting(
        json!({
            "cmd": "enhance_locator",
            "request_id": "browser-enhance",
            "selector": args.selector,
        }),
        &args.target,
    )?;
    let response = send_browser_command(project_root, command, timeout, true)?;
    let ok = response
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !ok {
        let error = response
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        anyhow::bail!("enhance failed: {error}");
    }
    print_json_response(response)
}

fn heal_execute(project_root: &Path, args: &BrowserExecuteArgs) -> Result<()> {
    if args.target.session.is_some() {
        return execute(project_root, args);
    }
    let selector = args
        .selector
        .as_deref()
        .ok_or_else(|| anyhow!("heal-execute currently requires --selector"))?;
    let timeout = command_timeout_for_ms(args.timeout_ms + 15_000);
    let command = apply_targeting(
        json!({
            "cmd": "heal_execute_locator",
            "request_id": "browser-heal-execute",
            "selector": selector,
            "action": args.action,
            "value": args.value_arg,
            "timeout_ms": args.timeout_ms,
        }),
        &args.target,
    )?;
    let response = send_browser_command(project_root, command, timeout, true)?;
    let ok = response
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !ok {
        let error = response
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        anyhow::bail!("heal_execute failed: {error}");
    }
    if response
        .get("healed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        eprintln!(
            "  healed: original={} → {}",
            response
                .get("original_selector")
                .and_then(|v| v.as_str())
                .unwrap_or("?"),
            response
                .get("selector")
                .and_then(|v| v.as_str())
                .unwrap_or("?"),
        );
    }
    print_json_response(response)
}

fn replay(project_root: &Path, args: &BrowserReplayArgs) -> Result<()> {
    let feature = match args.feature.as_deref() {
        Some(feature) => feature.replace('\\', "/"),
        None => {
            read_active_step(project_root)
                .context("read .teshi/active-step.json for default feature")?
                .feature_relative_path
        }
    };
    let steps = resolve_step_bindings(project_root, &feature, args.until_line)?;
    if steps.is_empty() {
        return Err(anyhow!("no confirmed bindings found for {feature}"));
    }

    let mut screenshot_entries: Vec<ReplayScreenshotEntry> = Vec::new();
    let screenshot_dir = project_root
        .join(".teshi")
        .join("logs")
        .join("replay-screenshots");
    let _ = fs::create_dir_all(&screenshot_dir);

    let non_interactive = args.non_interactive || args.yes;
    for (idx, step) in steps.iter().enumerate() {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "index": idx + 1,
                "total": steps.len(),
                "step_line": step.step_line,
                "step": format!("{} {}", step.step_keyword, step.step_text),
                "action": step.primary.action,
                "target": step.primary.value,
                "value_arg": step.primary.value_arg
            }))?
        );
        if args.dry_run {
            continue;
        }
        if !non_interactive {
            prompt_continue(step)?;
        }
        let response = if step.primary.action == "navigate" && args.target.session.is_some() {
            let (target, lease_token) = required_target(&args.target)?;
            let url = step
                .primary
                .value_arg
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(&step.primary.value);
            execute_typed_operation_value(
                project_root,
                BrowserOperation::NavigateBrowser {
                    target,
                    lease_token,
                    url: url.to_string(),
                    timeout_ms: 15_000,
                    wait: Some(BrowserWaitCondition::LoadComplete),
                    monitor: false,
                },
                command_timeout_for_ms(15_000),
            )?
        } else if step.primary.action == "navigate" {
            let url = step
                .primary
                .value_arg
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(&step.primary.value);
            let timeout_ms = 15_000;
            navigate_to_url(
                project_root,
                url,
                timeout_ms,
                command_timeout_for_ms(timeout_ms),
                &format!("browser-replay-{}", idx + 1),
                true,
                Some(&args.target),
            )?
        } else if step.primary.action == "open_project" {
            let path = step
                .primary
                .value_arg
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(&step.primary.value);
            let timeout_ms = 15_000;
            open_project_via_sidecar(
                project_root,
                path,
                command_timeout_for_ms(timeout_ms),
                &format!("browser-replay-{}", idx + 1),
                Some(&args.target),
            )?
        } else if args.target.session.is_some() {
            let (target, lease_token) = required_target(&args.target)?;
            let action: BrowserAction = serde_json::from_value(json!(step.primary.action))
                .with_context(|| format!("unsupported binding action {}", step.primary.action))?;
            let candidate = step
                .primary
                .structured_candidate
                .clone()
                .map(serde_json::from_value)
                .transpose()
                .context("binding structured_candidate is invalid")?;
            let element = BrowserElementInput {
                reference: step.primary.element_reference.clone(),
                candidate,
                css: (step.primary.element_reference.is_none()
                    && step.primary.structured_candidate.is_none())
                .then(|| step.primary.value.clone()),
                snapshot_id: None,
                page_context_revision: step
                    .primary
                    .page_context_revision
                    .clone()
                    .map(PageContextRevision),
            };
            execute_typed_operation_value(
                project_root,
                BrowserOperation::ExecuteBrowserAction {
                    target,
                    lease_token,
                    action,
                    element,
                    value: step.primary.value_arg.clone(),
                    files: vec![],
                    wait: None,
                    timeout_ms: 5_000,
                    focus: false,
                    monitor: false,
                },
                command_timeout_for_ms(5_000),
            )?
        } else {
            let timeout_ms = 5_000;
            execute_locator(
                project_root,
                ExecuteLocatorParams {
                    selector: &step.primary.value,
                    action: &step.primary.action,
                    value: step.primary.value_arg.as_deref(),
                    timeout_ms,
                    request_id: &format!("browser-replay-{}", idx + 1),
                    health_check: true,
                    target: Some(&args.target),
                },
                command_timeout_for_ms(timeout_ms),
            )?
        };
        // Capture screenshot after each step (before ensure_ok so we capture even on failure).
        if !args.dry_run {
            match capture_replay_step_screenshot(
                project_root,
                &feature,
                step.step_line,
                &step.step_keyword,
                &step.step_text,
                &screenshot_dir,
                &args.target,
            ) {
                Ok(entry) => {
                    println!(
                        "{}",
                        serde_json::to_string(&json!({
                            "event": "replay_screenshot",
                            "step_line": entry.step_line,
                            "screenshot_file": entry.screenshot_file,
                            "screenshots_dir": screenshot_dir.to_string_lossy(),
                        }))?
                    );
                    screenshot_entries.push(entry);
                }
                Err(e) => eprintln!(
                    "warning: screenshot capture failed at L{}: {e}",
                    step.step_line
                ),
            }
        }
        ensure_ok(&response).with_context(|| {
            format!(
                "replay failed at line {}: {} {}",
                step.step_line, step.step_keyword, step.step_text
            )
        })?;
    }

    if !args.dry_run && !screenshot_entries.is_empty() {
        let mut index = load_or_create_index(&screenshot_dir, &feature);
        index.steps = screenshot_entries;
        index.completed_at = Some(iso_now());
        index.status = "completed".to_string();
        if let Err(e) = save_index(&screenshot_dir, &index) {
            eprintln!("warning: failed to write screenshot index: {e}");
        }
        println!(
            "{}",
            serde_json::to_string(&json!({
                "event": "replay_complete",
                "screenshots_saved": index.steps.len(),
                "screenshots_dir": screenshot_dir.to_string_lossy(),
            }))?
        );
    }

    Ok(())
}

/// Starts the embedded Playwright sidecar and blocks until interrupted (for CI/scripts).
fn serve_embedded(args: &BrowserServeEmbeddedArgs) -> Result<()> {
    let rt = tokio::runtime::Runtime::new().context("create tokio runtime")?;
    rt.block_on(serve_embedded_async(args))
}

async fn serve_embedded_async(args: &BrowserServeEmbeddedArgs) -> Result<()> {
    let project_root = match &args.project {
        Some(path) => path.clone(),
        None => std::env::current_dir().context("resolve current directory")?,
    };
    let project_root = project_root
        .canonicalize()
        .with_context(|| format!("canonicalize project {}", project_root.display()))?;

    let dev_browser_script = resolve_embedded_browser_service_script(&project_root);
    // Headless CI: use repo browser_service.py and disable JPEG preview.
    // SAFETY: set before constructing runtime / spawning the Python sidecar child.
    unsafe {
        std::env::set_var("TESHI_BROWSER_SERVICE", &dev_browser_script);
    }

    let runtime = TeshiEngine::new(
        RuntimeConfig {
            browser_service_script: dev_browser_script,
            winapp_service_script: default_winapp_service_script(),
            embedded_no_preview_stream: false,
            requirements_root: None,
        },
        None,
    );

    open_project(runtime.clone(), project_root.to_string_lossy().to_string())
        .await
        .map_err(|e| anyhow!("open project: {e}"))?;

    let start = start_browser_sidecar(runtime.clone(), BrowserMode::Embedded)
        .await
        .map_err(browser_error)?;

    eprintln!("embedded sidecar ws_url={}", start.ws_url);
    eprintln!("cdp endpoint={}", start.cdp_endpoint_path);

    // Ensure cdp-endpoint.json is written from the Rust side with the actual ws_url,
    // so subsequent commands (e.g. navigate) don't race with the Python sidecar's write.
    if let Err(e) =
        write_cdp_endpoint_from_rust(&project_root, &start.ws_url, &start.mode, "about:blank")
    {
        eprintln!("warning: failed to write cdp-endpoint.json: {e}");
    }

    if let Some(url) = args.navigate.as_deref() {
        let timeout_ms = 15_000;
        let response = navigate_to_url(
            &project_root,
            url,
            timeout_ms,
            command_timeout_for_ms(timeout_ms),
            "serve-embedded-navigate",
            false,
            None,
        )?;
        ensure_ok(&response).with_context(|| format!("navigate to {url}"))?;
        eprintln!("navigated to {url}");
    }

    eprintln!("embedded sidecar running; press Ctrl+C to stop");
    tokio::signal::ctrl_c().await.context("wait for Ctrl+C")?;
    stop_browser_sidecar(&runtime)
        .await
        .map_err(|e| anyhow!("stop sidecar: {e}"))?;
    Ok(())
}

fn browser_error(err: teshi_engine::BrowserError) -> anyhow::Error {
    if let Some(hint) = err.hint {
        anyhow!("{} — {hint}", err.message)
    } else {
        anyhow!(err.message)
    }
}

/// Prefers the repo `browser_service.py` so CI picks up the latest embedded flags.
fn resolve_embedded_browser_service_script(project_root: &Path) -> PathBuf {
    let repo_script = project_root.join("resources/browser_service.py");
    if repo_script.is_file() {
        return repo_script;
    }
    default_browser_service_script()
}

fn prompt_continue(step: &StepBinding) -> Result<()> {
    eprint!(
        "About to run L{}: {} {}. Press Enter to continue, or Ctrl+C to stop.",
        step.step_line, step.step_keyword, step.step_text
    );
    io::stderr().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(())
}

/// Capture one replay screenshot: Chrome uses P1 `capture_browser_screenshot`;
/// embedded/WinApp still use the sidecar `cmd: screenshot` JPEG.
fn capture_replay_step_screenshot(
    project_root: &Path,
    feature: &str,
    step_line: usize,
    step_keyword: &str,
    step_text: &str,
    screenshot_dir: &Path,
    target: &BrowserTargetArgs,
) -> Result<ReplayScreenshotEntry> {
    let feature_sanitized = teshi_engine::sanitize_feature_path(feature);
    if target.session.is_some() {
        let (browser_target, lease_token) = required_target(target)?;
        let endpoint = read_cdp_endpoint(project_root)?;
        let client = BrowserOperations::new(endpoint.ws_url, Duration::from_secs(15))
            .with_caller_label("teshi-cli")
            .with_project_root(project_root.to_string_lossy());
        let response = client
            .execute(&BrowserOperation::CaptureBrowserScreenshot {
                target: browser_target,
                lease_token,
                page_context_revision: None,
                format: BrowserScreenshotFormat::Jpeg,
                quality: Some(70),
                full_page: false,
                element: None,
            })
            .map_err(|error| anyhow!("{}", error.message))?;
        let artifact = artifact_path_from_screenshot_payload(&response.payload)?;
        save_screenshot_from_artifact(
            &artifact,
            &feature_sanitized,
            step_line,
            step_keyword,
            step_text,
            screenshot_dir,
        )
    } else {
        let endpoint = read_cdp_endpoint(project_root)?;
        capture_and_save_screenshot(
            &endpoint.ws_url,
            project_root,
            &feature_sanitized,
            step_line,
            step_keyword,
            step_text,
            screenshot_dir,
        )
    }
}

/// Sidecar wait budget: locator timeout plus slack for Chrome heartbeat and CDP work.
fn command_timeout_for_ms(timeout_ms: u64) -> Duration {
    let secs = timeout_ms.div_ceil(1000).saturating_add(5);
    Duration::from_secs(secs.max(15))
}

fn required_target(args: &BrowserTargetArgs) -> Result<(BrowserTarget, String)> {
    let session = args
        .session
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("--session is required for this browser operation"))?;
    let window_id = args
        .window
        .ok_or_else(|| anyhow!("--window is required for this browser operation"))?;
    let tab_id = args
        .tab
        .ok_or_else(|| anyhow!("--tab is required for this browser operation"))?;
    let lease_token = args
        .lease_token
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("--lease-token is required for this browser operation"))?;
    Ok((
        BrowserTarget {
            extension_instance_id: session.to_string(),
            window_id,
            tab_id,
        },
        lease_token.to_string(),
    ))
}

fn apply_targeting(
    mut command: serde_json::Value,
    args: &BrowserTargetArgs,
) -> Result<serde_json::Value> {
    let supplied = [
        args.session.is_some(),
        args.window.is_some(),
        args.tab.is_some(),
        args.lease_token.is_some(),
    ];
    if supplied.iter().all(|value| !value) {
        return Ok(command);
    }
    let (target, lease_token) = required_target(args)?;
    let object = command
        .as_object_mut()
        .ok_or_else(|| anyhow!("browser command must be a JSON object"))?;
    object.insert("target".to_string(), serde_json::to_value(target)?);
    object.insert("lease_token".to_string(), json!(lease_token));
    Ok(command)
}

fn navigate_to_url(
    project_root: &Path,
    url: &str,
    timeout_ms: u64,
    sidecar_timeout: Duration,
    request_id: &str,
    health_check: bool,
    target: Option<&BrowserTargetArgs>,
) -> Result<serde_json::Value> {
    let command = json!({
        "cmd": "navigate",
        "request_id": request_id,
        "url": url,
        "timeout_ms": timeout_ms
    });
    send_browser_command(
        project_root,
        match target {
            Some(target) => apply_targeting(command, target)?,
            None => command,
        },
        sidecar_timeout,
        health_check,
    )
}

fn open_project_via_sidecar(
    project_root: &Path,
    path: &str,
    sidecar_timeout: Duration,
    request_id: &str,
    target: Option<&BrowserTargetArgs>,
) -> Result<serde_json::Value> {
    let command = json!({
        "cmd": "open_project",
        "request_id": request_id,
        "path": path
    });
    send_browser_command(
        project_root,
        match target {
            Some(target) => apply_targeting(command, target)?,
            None => command,
        },
        sidecar_timeout,
        true,
    )
}

/// Bundles parameters for a single `execute_locator` sidecar command.
struct ExecuteLocatorParams<'a> {
    selector: &'a str,
    action: &'a str,
    value: Option<&'a str>,
    timeout_ms: u64,
    request_id: &'a str,
    health_check: bool,
    target: Option<&'a BrowserTargetArgs>,
}

fn execute_locator(
    project_root: &Path,
    params: ExecuteLocatorParams<'_>,
    sidecar_timeout: Duration,
) -> Result<serde_json::Value> {
    let command = json!({
        "cmd": "execute_locator",
        "request_id": params.request_id,
        "selector": params.selector,
        "action": params.action,
        "value": params.value,
        "timeout_ms": params.timeout_ms
    });
    send_browser_command(
        project_root,
        match params.target {
            Some(target) => apply_targeting(command, target)?,
            None => command,
        },
        sidecar_timeout,
        params.health_check,
    )
}

fn send_browser_command(
    project_root: &Path,
    command: serde_json::Value,
    timeout: Duration,
    health_check: bool,
) -> Result<serde_json::Value> {
    let started = Instant::now();
    let cmd = command
        .get("cmd")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let request_id = command
        .get("request_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if health_check && auto_reconnect_enabled() {
        let _ = ensure_sidecar_healthy(project_root);
    }
    let endpoint = read_cdp_endpoint(project_root)?;
    debug_log(
        project_root,
        json!({
            "event": "browser_command_start",
            "cmd": cmd.clone(),
            "request_id": request_id.clone(),
            "command": command.clone(),
            "timeout_ms": timeout.as_millis(),
            "endpoint_path": endpoint.endpoint_path.display().to_string(),
            "project_root": endpoint.project_root.display().to_string(),
        }),
    );
    let ws_url = endpoint.ws_url;
    match send_sidecar_command_with_timeout(&ws_url, command, timeout).map_err(anyhow::Error::msg) {
        Ok(response) => {
            debug_log(
                project_root,
                json!({
                    "event": "browser_command_end",
                    "cmd": cmd.clone(),
                    "request_id": request_id.clone(),
                    "elapsed_ms": started.elapsed().as_millis(),
                    "ok": response.get("ok").and_then(|v| v.as_bool()),
                    "error": response.get("error")
                }),
            );
            Ok(response)
        }
        Err(err) => {
            debug_log(
                project_root,
                json!({
                    "event": "browser_command_error",
                    "cmd": cmd.clone(),
                    "request_id": request_id.clone(),
                    "elapsed_ms": started.elapsed().as_millis(),
                    "error": err.to_string()
                }),
            );
            Err(err)
        }
    }
}

fn ensure_ok(response: &serde_json::Value) -> Result<()> {
    if response.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        return Ok(());
    }
    let error = response
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("browser command failed");
    Err(anyhow!("{error}"))
}

fn print_json_response(response: serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&response)?);
    if let Some(error) = json_response_error(&response) {
        return Err(anyhow!(error));
    }
    Ok(())
}

fn json_response_error(response: &serde_json::Value) -> Option<String> {
    if response.get("ok").and_then(|value| value.as_bool()) == Some(false) {
        let code = response
            .get("code")
            .and_then(|value| value.as_str())
            .unwrap_or("browser_operation_failed");
        let error = response
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or("browser operation failed");
        return Some(format!("{code}: {error}"));
    }
    None
}

fn debug_log(project_root: &Path, mut payload: serde_json::Value) {
    if std::env::var_os("TESHI_BROWSER_DEBUG").is_none() {
        return;
    }
    if let serde_json::Value::Object(ref mut object) = payload {
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default();
        object.insert("ts_ms".to_string(), json!(ts_ms));
    }
    redact_sensitive_json(&mut payload);
    let log_dir = project_root.join(".teshi").join("logs");
    if fs::create_dir_all(&log_dir).is_err() {
        return;
    }
    let path = log_dir.join("cli-browser.log");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{}", payload);
    }
}

fn redact_sensitive_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            object.retain(|key, nested| {
                let normalized = key.to_ascii_lowercase().replace('-', "_");
                let secret = normalized == "lease_token"
                    || normalized == "capability_grant"
                    || normalized == "capability_grant_token"
                    || normalized.ends_with("_secret");
                if !secret {
                    redact_sensitive_json(nested);
                }
                !secret
            });
        }
        serde_json::Value::Array(values) => {
            for value in values {
                redact_sensitive_json(value);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_json_operation_failure_becomes_a_cli_error() {
        let response = json!({
            "ok": false,
            "code": "ambiguous_browser_target",
            "error": "select a browser profile",
        });
        assert_eq!(
            json_response_error(&response).as_deref(),
            Some("ambiguous_browser_target: select a browser profile")
        );
    }

    #[test]
    fn successful_json_operation_has_no_cli_error() {
        assert_eq!(json_response_error(&json!({"ok": true})), None);
    }

    #[test]
    fn cli_diagnostic_json_removes_lease_and_grant_secrets() {
        let mut payload = json!({
            "event": "browser_command_start",
            "command": {
                "lease_token": "lease-private",
                "target": {"extension_instance_id": "profile-a"},
                "nested": {
                    "capability_grant_token": "grant-private",
                    "retryable": false
                }
            },
            "recovery": {
                "owner_label": "agent-a",
                "broker_secret": "broker-private"
            }
        });
        redact_sensitive_json(&mut payload);
        let serialized = payload.to_string();
        assert!(!serialized.contains("lease-private"));
        assert!(!serialized.contains("grant-private"));
        assert!(!serialized.contains("broker-private"));
        assert!(serialized.contains("profile-a"));
        assert!(serialized.contains("retryable"));
    }

    #[test]
    fn filtered_network_capture_fails_closed_for_legacy_capabilities() {
        let discovery = json!({
            "sessions": [{
                "identity": {"extension_instance_id": "profile-a"},
                "capabilities": {
                    "features": [{
                        "feature": "p1.observability_artifacts",
                        "available": true
                    }]
                }
            }]
        });
        let error =
            validate_filtered_network_capture_capabilities(&discovery, "profile-a").unwrap_err();
        assert_eq!(
            error.code,
            BrowserAgentErrorCode::BrowserCapabilityUnavailable
        );
        assert_eq!(
            error.recovery["missing_features"],
            json!(["p1.filtered_network_capture", "p1.network_batch_transport"])
        );
    }

    #[test]
    fn filtered_network_capture_accepts_both_negotiated_capabilities() {
        let discovery = json!({
            "sessions": [{
                "identity": {"extension_instance_id": "profile-a"},
                "capabilities": {
                    "features": [
                        {"feature": "p1.filtered_network_capture", "available": true},
                        {"feature": "p1.network_batch_transport", "available": true}
                    ]
                }
            }]
        });
        assert!(validate_filtered_network_capture_capabilities(&discovery, "profile-a").is_ok());
    }
}
