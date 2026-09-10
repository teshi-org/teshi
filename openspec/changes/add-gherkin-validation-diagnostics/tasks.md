## 1. Core Diagnostic Model

- [x] 1.1 Add serializable diagnostic severity, stable code, source location, suggestion, summary, and report types to `teshi-core`
- [x] 1.2 Add a pure per-source validation entry point and project-report aggregation API without filesystem or shell dependencies
- [x] 1.3 Define deterministic diagnostic ordering and tests for empty, valid, and multi-error reports

## 2. Source-Aware Gherkin Validation

- [x] 2.1 Implement active-dialect keyword-prefix detection using `gherkin_lang` data and longest-keyword matching
- [x] 2.2 Diagnose missing step separators with exact line, column, stable `missing_step_separator` code, and insertion suggestion
- [x] 2.3 Diagnose malformed Feature/Rule/Background/Scenario/Examples structure and unrecognized executable-region lines
- [x] 2.4 Preserve valid Feature and Scenario prose, comments, tags, data tables, and DocStrings during validation
- [x] 2.5 Add English, `zh-CN`, shared-prefix dialect, Unicode, comments, prose, table, DocString, and malformed-structure regression tests
- [x] 2.6 Verify malformed source still produces the existing permissive partial AST while validation independently reports errors

## 3. Consolidate Existing Validation

- [x] 3.1 Move or adapt the existing `teshi-agent` Given/When/Then, duplicate-name, Examples, step-count, and scenario-dependency checks into the core diagnostics model
- [x] 3.2 Replace `teshi-agent::validator` rule ownership with a compatibility adapter over `teshi-core`
- [x] 3.3 Update the agent `validate_feature` tool and tests to return core-derived diagnostics without maintaining dialect syntax rules
- [x] 3.4 Add an architecture regression check proving non-core crates and Skills contain no duplicate Gherkin keyword rule table

## 4. Canonical Check CLI

- [x] 4.1 Add the top-level `teshi check` command with mutually exclusive `--feature <path>` and `--all` scope options
- [x] 4.2 Implement project and single-file discovery adapters that read source and call the pure core validator
- [x] 4.3 Implement human-readable output with path, line, column, severity, code, message, suggestion, and aggregate counts
- [x] 4.4 Implement the stable `--json` report envelope and deterministic diagnostic ordering
- [x] 4.5 Implement distinct successful, validation-error, and invocation/I/O exit behavior, with warnings alone remaining successful
- [x] 4.6 Add CLI integration tests for default scope, explicit Feature, all Features, JSON output, multilingual errors, warnings-only output, missing files, and conflicting flags

## 5. Binding and Execution Gates

- [x] 5.1 Add a shared selected-scope validation preflight usable by direct-file and daemon-backed command paths
- [x] 5.2 Gate `steps unbound` so an invalid Feature returns diagnostics instead of a successful empty or partial list
- [x] 5.3 Gate `steps next-unbound` and selection/resolution paths before they write active-step or binding state
- [x] 5.4 Gate `teshi run` before runner startup when any selected Feature has error diagnostics
- [x] 5.5 Gate browser and WinApp replay before executing any bound target action
- [x] 5.6 Add direct-versus-daemon parity tests and assert failed validation performs no binding mutation, runner launch, or target action

## 6. Editor Diagnostics

- [x] 6.1 Expose buffer-source diagnostics through the shared editor/backend contract without changing permissive parsing
- [x] 6.2 Present current-buffer diagnostics in the TUI while preserving Browse/Insert editing, navigation, and save behavior
- [x] 6.3 Expose equivalent diagnostics to Desktop/Web backends and render non-blocking error locations and suggestions
- [x] 6.4 Add editor tests proving incomplete Feature content remains visible and editable while diagnostics update with buffer changes

## 7. Skills, CI, and Documentation

- [x] 7.1 Update the Teshi and `bdd-feature` Skills to call `teshi check --feature ... --json` before binding or execution and remove duplicated dialect syntax rules
- [x] 7.2 Add repository CI coverage using `teshi check --all` without replacing existing Rust and Web quality gates
- [x] 7.3 Document check scopes, JSON schema, diagnostic severities/codes, exit behavior, execution gates, and editor permissiveness
- [x] 7.4 Add paired English and Chinese acceptance Features for missing separators, legal prose, command output, and binding/execution blocking

## 8. Validation

- [x] 8.1 Run focused `teshi-core`, `teshi-agent`, CLI steps/run, daemon, TUI, and shared UI test suites
- [x] 8.2 Run `cargo fmt --all --check`, native workspace check/test excluding `teshi-web`, and strict workspace Clippy
- [x] 8.3 Run the separate GPUI Web smoke gate after Desktop/Web diagnostics integration
- [x] 8.4 Run `openspec validate add-gherkin-validation-diagnostics --strict` and reconcile every implemented requirement with automated evidence

## 9. Review Follow-up Hardening

- [x] 9.1 Keep legal repeated `Given`/`And` scenarios non-blocking and add Core/Agent/CLI regression coverage
- [x] 9.2 Restrict directory execution validation to the selected directory and add a runner regression test
- [x] 9.3 Transport daemon warning reports through REST/control/NDJSON paths and add warning-availability tests
- [x] 9.4 Preserve structured validation reports in daemon REST/control errors and add response-shape tests
- [x] 9.5 Use the daemon's authoritative project root for daemon-backed steps routing and add a routing regression test

## 10. Feature Traceability Hardening

- [x] 10.1 Add paired executable Feature scenarios for repeated Given steps, selected-directory execution scope, daemon warning/error payloads, valid multilingual input, and unrecognized executable text
- [x] 10.2 Execute the added CLI cases through the bilingual validation self-bootstrap runner
- [x] 10.3 Remove non-executable Core, architecture, CI-packaging, and protocol prose from `features/`; enforce explicit runner ownership for the remaining product Features
