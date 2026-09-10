## MODIFIED Requirements

### Requirement: `teshi-core` owns all Gherkin concepts

The `teshi-core` crate SHALL be the single source of truth for Gherkin parsing, language keywords, syntax highlighting, render payloads, step indexing, validation rules, and diagnostic types. No other crate, application shell, packaged Skill, or CI script SHALL contain a duplicate Gherkin parser or dialect syntax validator; consumers SHALL call or adapt the `teshi-core` implementation.

#### Scenario: No Gherkin duplication in root
- **WHEN** the target structure is in place
- **THEN** `src/gherkin.rs`, `src/gherkin_lang.rs`, `src/highlight.rs`, and any root Gherkin validator do NOT exist or are thin re-exports of `teshi_core`

#### Scenario: No Gherkin duplication in engine
- **WHEN** `crates/teshi-engine/src/gherkin.rs` exists
- **THEN** it contains only I/O and adapter functions and calls into `teshi_core` for parsing, rendering, validation, and diagnostics

#### Scenario: Agent validation delegates to core
- **WHEN** `teshi-agent` exposes a Feature validation tool or authoring guidance
- **THEN** its validation result SHALL be derived from `teshi-core` diagnostics and it SHALL NOT maintain an independent dialect keyword or syntax rule table

#### Scenario: Skills and CI validate Feature syntax
- **WHEN** a Teshi Skill or CI workflow requires Feature correctness
- **THEN** it SHALL invoke the Teshi validation interface rather than reimplementing Gherkin syntax checks
