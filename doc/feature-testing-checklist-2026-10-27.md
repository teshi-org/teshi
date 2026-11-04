# Feature testing checklist

Scope: all product Features; statuses reflect actual execution, not syntax checks. Unchecked cases remain open; tracked details and artifacts in feature-test-followup-2026-10-27.md.

| Feature | Scenarios | State |
|---|---:|---|
| existing browser_profiles.feature | 5 | open: verify current binary and bilingual scenarios |
| doc-26 chat_acp_status_row.feature | 5 | open: verify current binary and bilingual scenarios |
| doc-29 chat_focus_hint.feature | 2 | bilingual 1 passed, 1 failed: legacy PTY maps Shift+Enter to Enter; chat-focus-feature-20261027 |
| doc-25 chat_profile_title.feature | 3 | passed: bilingual 3/3 PTY case-level replay, chat-title-feature-20261027 |
| doc-27 chat_runtime_context.feature | 3 | passed: bilingual 3/3 PTY case-level replay, chat-runtime-feature-rerun-20261027 |
| doc-36 combined_detail_layout.feature | 2 | passed: bilingual 2/2 compact-layout-feature-20261027 |
| doc-31 compact_terminal_layout.feature | 2 | partial: bilingual 1/2, resize pointer pending |
| doc-33 explore_horizontal_navigation.feature | 2 | passed: bilingual 2/2 PTY case-level replay, horizontal-feature-rerun-20261027 |
| doc-32 explore_step_changes.feature | 2 | open: verify current binary and bilingual scenarios |
| doc-30 external_feature_sync.feature | 5 | partial: bilingual 3/5 passed, 2 unbound, external-context-20261027 |
| doc-34 footer_actions.feature | 2 | passed: bilingual 2/2 PTY case-level replay, footer-feature-final-20261027 |
| existing gherkin_validation_e2e.feature | 17 | passed: bilingual replay cli-matrix-current-20261027 |
| doc-35 pointer_feedback.feature | 2 | partial: bilingual 1/2, modal coverage pending |
| existing replay_screenshot_review.feature | 10 | open: verify current binary and bilingual scenarios |
| existing requirement_authoring.feature | 9 | passed: bilingual replay cli-matrix-current-20261027 |
| doc-23 requirement_creation.feature | 5 | passed: bilingual replay cli-matrix-current-20261027 |
| existing requirement_listing.feature | 13 | passed: bilingual replay cli-matrix-current-20261027 |
| existing requirement_tab.feature | 1 | passed: bilingual replay cli-matrix-current-20261027 |
| doc-24 requirement_timestamp_names.feature | 4 | open: verify current binary and bilingual scenarios |
| existing run_inspect.feature | 7 | open: verify current binary and bilingual scenarios |
| doc-28 terminal_title_policy.feature | 2 | blocked: product decision pending |
| existing testpoint_requirement_links.feature | 12 | passed: bilingual replay cli-matrix-current-20261027 |
| existing tui_acp_authoring.feature | 1 | open: verify current binary and bilingual scenarios |
| existing tui_acp_long_conversation.feature | 1 | open: verify current binary and bilingual scenarios |
| existing tui_acp_model_persistence.feature | 5 | open: verify current binary and bilingual scenarios |
| existing tui_acp_permission_allow.feature | 1 | open: verify current binary and bilingual scenarios |
| existing tui_acp_permission_reject.feature | 1 | open: verify current binary and bilingual scenarios |
| existing tui_acp_settings.feature | 1 | open: verify current binary and bilingual scenarios |
| existing tui_acp_wrapped_chat.feature | 1 | open: verify current binary and bilingual scenarios |
| existing tui_agent_activity.feature | 13 | open: verify current binary and bilingual scenarios |
| existing tui_agent_selection_memory.feature | 9 | open: verify current binary and bilingual scenarios |
| existing tui_codebuddy_live_conversation.feature | 1 | open: verify current binary and bilingual scenarios |
| existing tui_conversation_history.feature | 14 | open: verify current binary and bilingual scenarios |
| existing tui_editor.feature | 2 | open: verify current binary and bilingual scenarios |
| existing tui_exit_performance.feature | 5 | open: verify current binary and bilingual scenarios |
| existing tui_project_switch.feature | 17 | open: verify current binary and bilingual scenarios |
| existing tui_project_switch_boundaries.feature | 7 | open: verify current binary and bilingual scenarios |
| existing tui_project_switch_failures.feature | 7 | open: verify current binary and bilingual scenarios |
| doc-22 winapp_capability_diagnostics.feature | 3 | open: verify current binary and bilingual scenarios |
| doc-20 winapp_owned_instances.feature | 3 | open: verify current binary and bilingual scenarios |
| doc-21 winapp_visual_exploration.feature | 9 | open: verify current binary and bilingual scenarios |

- [ ] Reconcile Chinese Feature pair and all scenario results; keep English and Chinese evidence separate.
- [ ] features/winapp/negative_assertion.feature: real attached target and replay.
- [ ] Existing replay screenshot Web pair: verified binding and GPUI/WebGPU replay.
- [ ] Feature catalog: all executable owners and genuine bindings.
- [ ] Full native workspace tests, Clippy, docs, formatting; record environment failures.
- [ ] Web smoke / GPUI WebGPU; Linux PTY and cross-platform coverage as applicable.
- [ ] WinApp owned instance, native window, Steam and three-Agent acceptance.
- [ ] Maintain source/binary identity, isolated stores and separate failure/blocked evidence.
