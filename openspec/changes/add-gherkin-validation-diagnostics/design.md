## Context

Teshi currently parses `.feature` source with a lightweight, intentionally permissive parser in `teshi-core`. That parser supports in-progress editor content, but an unrecognized line can disappear from the AST without a diagnostic. In particular, a dialect keyword prefix without the required separator, such as `当用户登录`, is not a step and can be treated as prose. Downstream step indexing then sees an empty or partial Scenario, allowing `teshi steps unbound` to return a misleading success.

There is already a separate validator in `teshi-agent`, but it receives only the parsed AST and checks authoring heuristics such as Given/When/Then ordering, duplicate names, and missing Examples. It cannot diagnose source lines discarded during parsing, and its location conflicts with the existing architecture rule that `teshi-core` owns Gherkin concepts. CLI steps commands also have direct-file and daemon-backed paths that must enforce identical preflight behavior.

## Goals / Non-Goals

**Goals:**

- Make `teshi-core` the single source of truth for source-aware Gherkin diagnostics across supported dialects.
- Preserve permissive parsing so editors can load and render incomplete documents.
- Distinguish definite syntax/structure errors from valid description prose and advisory authoring guidance.
- Provide stable human-readable and JSON CLI output with useful source locations and repair suggestions.
- Block step discovery/selection and BDD execution when their Feature scope contains error diagnostics.
- Reuse the same diagnostics from CLI, daemon, editor, agent, Skill, and CI entry points.

**Non-Goals:**

- Replacing the lightweight parser with Cucumber's parser or making every editor keystroke produce a parse failure.
- Treating all free text in Feature, Rule, Background, or Scenario blocks as invalid.
- Requiring warning-free Features before binding or execution.
- Defining project-specific style rules, selectors, bindings, runner fixtures, or cleanup policy as Gherkin syntax.
- Automatically rewriting Feature files.

## Decisions

### 1. Validation is a source-aware, pure `teshi-core` operation

`teshi-core` will expose a deterministic validation API that accepts source text, path identity, and validation options and returns a report. The report will contain diagnostics with at least `path`, one-based `line` and `column`, `severity`, stable `code`, localized or neutral `message`, and optional `suggestion`. Project validation will aggregate per-file reports without performing filesystem I/O inside core.

The validator will inspect both source-line context and the permissively parsed AST. Source inspection is required for text that never reaches the AST; AST inspection remains useful for structural and authoring checks. Existing `teshi-agent` validation behavior will move or delegate to this API so there is one rule implementation.

Alternative: make the parser return `Result` and reject malformed input. This was rejected because partial Feature content is normal while editing and must remain renderable.

### 2. Diagnostics use stable codes and severity-based gating

Definite failures will be `error`, including recognized dialect step-keyword prefixes missing a required separator, missing or malformed required structure, invalid attachment context, and other cases where intended executable structure cannot be represented safely. Advisory quality checks remain `warning` or `suggestion`. In particular, Given/When/Then completeness is authoring guidance rather than generic Gherkin syntax: a scenario may legally contain repeated `Given`/`And` steps, so missing `When` or `Then` diagnostics must not block binding or execution.

The missing-separator detector will use the active dialect data from `gherkin_lang`, prefer the longest matching keyword, and avoid hard-coded Chinese or English lists. A line beginning with a dialect step keyword immediately followed by non-whitespace will produce `missing_step_separator` with a suggested insertion. Valid comments, tags, blank lines, tables, DocStrings, and allowed description positions will not trigger this error.

Scenario prose before the first recognized step remains legal. After step parsing has begun, otherwise unrecognized non-attachment text will be diagnosed because silently dropping it would make executable intent ambiguous. Lower-confidence suspicious text that is not a keyword-prefix match can remain a warning.

Alternative: put spacing rules in Skills or CI scripts. This was rejected because it duplicates dialect knowledge and allows CLI behavior to disagree with automation.

### 3. `teshi check` is the canonical command adapter

The CLI will add a top-level `check` command. With no selector it checks the current project scope; `--feature <path>` checks one Feature; `--all` explicitly checks all discovered Feature files. `--feature` and `--all` are mutually exclusive. `--json` emits one stable envelope containing summary counts and ordered diagnostics.

Human output will show `path:line:column`, severity, code, message, and suggestion. Exit status will be zero when no errors exist, nonzero when validation errors exist, and distinguish invocation/I/O failure from a completed invalid report. Warnings alone do not fail the command.

Alternative: expose validation only through the existing agent tool. This was rejected because CI, users, and non-agent integrations need the same contract.

### 4. Consumers gate on core reports without changing parser behavior

Commands that derive executable steps or run a Feature will validate exactly the selected scope before reading the step index or starting a runner. At minimum this includes `steps unbound`, `steps next-unbound`, step selection/resolution paths that parse a Feature, `teshi run`, and browser/WinApp replay entry points. If errors exist, the command returns the same diagnostics contract and performs no binding-state mutation or test execution.

Daemon-backed and direct-file paths will call shared validation/preflight code so a live daemon cannot change semantics. Warnings remain visible but do not block. Existing REST or WebSocket adapters will transport reports rather than reimplement rules; REST responses use an additive validation-report header or NDJSON validation event, while control responses carry the report in the protocol's structured error/details fields or an associated validation event.

Alternative: require users and Skills to run `teshi check` manually first. This was rejected because callers can skip the command and recreate the current false-success behavior.

### 5. Editors consume diagnostics independently from parsing

TUI, Desktop, and Web editor surfaces will continue parsing and rendering the current buffer even when it has errors. They may request diagnostics on buffer changes and present inline or panel feedback, but validation does not prevent typing, navigation, or saving unless a separate product decision explicitly introduces such a policy. Any debounce or background scheduling belongs to the shell; diagnostic computation remains pure.

This separates two contracts: permissive parsing supports authoring, while error-free validation is required only at execution and binding boundaries.

### 6. Agent and Skill behavior delegates to Teshi

The existing `validate_feature` agent tool will become an adapter over core diagnostics and retain compatibility where practical. Packaged Skills will instruct agents to call `teshi check --feature ... --json` before binding or execution and will not enumerate dialect syntax rules. CI will use the same command and exit semantics.

## Risks / Trade-offs

- [Valid prose may resemble a malformed step] → Reserve hard errors for exact active-dialect keyword-prefix evidence and well-defined post-step contexts; cover legitimate descriptions in tests.
- [New gates expose previously ignored malformed files] → Provide precise suggestions, scope validation to the requested Feature where possible, and introduce the command before enabling all gates.
- [Direct and daemon paths drift] → Put report generation and gate decisions in shared core/engine-facing helpers and run parity tests for both routes.
- [Diagnostics become unstable for automation] → Treat codes, locations, severity values, JSON schema, and ordering as versioned CLI contracts.
- [Editor validation on every keystroke is expensive] → Keep validation linear in source size and let shells debounce or cancel stale computations.

## Migration Plan

1. Add core diagnostic types, source scanner, structural validation, and multilingual regression tests without changing parser callers.
2. Move/delegate existing agent best-practice validation to core and preserve the agent tool output contract.
3. Add `teshi check` human/JSON adapters and CLI integration tests, then document CI usage.
4. Add shared preflight gates to direct and daemon-backed step operations and BDD/replay execution paths.
5. Expose reports to editor backends and add non-blocking presentation while retaining permissive rendering.
6. Simplify Teshi Skills to call the CLI validator and remove duplicated syntax guidance.

Rollback can disable consumer gates while retaining the additive `teshi check` command and core diagnostics. Parser behavior and existing Feature files remain unchanged throughout migration.

## Open Questions

- Whether `teshi check` with no flags should be a documented alias for `--all` or use only the currently selected Feature when project state provides one.
- Which editor surface should receive inline diagnostics first if TUI, Desktop, and Web cannot land atomically.
- Whether the first release should expose suggestion-level authoring heuristics in JSON by default or behind an explicit verbosity option.
