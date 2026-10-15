## Context

`AcpClient::new_session` and `load_session` currently retain only ID and cwd. The runtime worker accepts Prompt/Cancel/Shutdown and ignores generic SessionUpdate events. The TUI renders a static active_model_label; approval policy is captured when the backend is configured. These omissions prevent runtime parameter controls even when the agent advertises them.

References inspected: pocketbot/dev/lib/services/websocket_service.dart (metadata application and setConfigOption), pocketbot/dev/lib/screens/chat_screen.dart (dynamic controls), and chrys/src/chrys/app/tui/widgets/chrome/status_bar.py (clickable selectors, responsive labels, disabled states). Reuse behavior, not framework-specific code or provider-specific guessed values.

## Goals / Non-Goals

**Goals:** expose every advertised configuration option; support grouped select and boolean values; use server-confirmed values; keep configuration scoped to its conversation; provide mouse and keyboard access before the first prompt.

**Non-Goals:** adding undocumented provider settings, implementing ACP v2 migration, changing native LLM settings, modifying the reference projects, persisting speculative defaults, or changing test-point review semantics.

## Decisions

1. Normalize ACP v1 configOptions into a typed session configuration snapshot. Preserve ID, name, description, category, current value, type, choices, and group labels. Unknown types remain visible but read-only with a reason. Categories are hints, not an allowlist. Prefer modern options per category and synthesize legacy model/mode controls only where the category is absent; route these to their actual legacy methods. Do not copy pocketbot's guessed Cursor mode list or route a synthetic legacy model to set_config_option.
2. Keep the ACP client responsible for wire requests and parsing. The runtime owns session state, pending mutation, and error state. Frontends use normalized values rather than JSON-RPC. New/load results, config_option_update, and current_mode_update feed the same state reducer, including notifications received while idle. Validate session identity and reject stale option/value selections.
3. Separate connecting from starting a prompt. Selecting an agent or opening settings can discover controls without sending a user message or marking a turn running. Retain the same remote session when changing settings. Serialize configuration mutations; keep the last confirmed snapshot until success. Apply complete returned option lists because changing a model can change the available reasoning levels. On rejection retain the snapshot and report a recoverable settings error. On timeout mark state uncertain and reconnect/reload before claiming a confirmed setting.
4. Expose settings in a bottom row adjacent to the composer: Agent, Mode, Model, Thinking, other advertised controls, and Teshi approvals. Use names supplied by the agent and a More entry when they do not fit. Every option remains reachable through a keyboard-accessible settings picker. Grouped lists scroll; boolean options expose On/Off; descriptions are visible in the picker. Pending controls cannot be activated again. During a turn, show current values but defer editing until idle.
5. Agent Mode and Teshi approvals remain independent. The latter controls responses to session/request_permission, not the agent sandbox. Changing the approval badge must update the effective client policy for subsequent requests; existing pending requests remain explicit decisions. Environment policy overrides must be reflected in the displayed effective value. Keep mandatory review gates independent.
6. No automatic persistence of model or permission defaults in this change. Reconnection discovers authoritative settings again. Native backend controls keep their existing behavior.

## Risks / Trade-offs

- Protocol and SDK evolution -> pin behavior to the checkout's negotiated v1 and verify wire shapes against the installed SDK and official protocol, including boolean support and legacy experimental model methods.
- Concurrent edits in app.rs/ui.rs -> keep new behavior in focused modules, re-read before integrating, preserve other edits.
- Delayed configuration responses -> track the owning session and revision, do not overwrite a newer conversation or claim unconfirmed values.
- Narrow terminals and wide Unicode labels -> measure terminal cells, retain overflow access, test minimum widths and mouse rectangles.
- Fake agents do not prove provider compatibility -> separate deterministic gates from real-agent and visible TUI acceptance.

## Migration Plan

Add protocol support and fixtures first, runtime controls second, then the TUI. Run affected tests and native compile/lint checks. Exercise a real configured agent and a real terminal before checking manual acceptance. No configuration migration is required; rollback removes the new controls without rewriting profiles.

## Open Questions

- User terminology "thinking rate" is provisionally interpreted as reasoning effort. Advertised speed options are included independently.
- Which locally available agents expose boolean options and grouped selects must be established by live discovery; fixtures cover both regardless.
