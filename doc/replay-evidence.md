# Replay evidence and acceptance

UI replay captures are runner responsibilities. Gherkin does not need explicit
screenshot steps. Web captures the tested page, WinApp captures the tested
window, and TUI captures the real PTY viewport as SVG, PNG preview, and text.
CLI and API-only steps explicitly report `not_applicable` and retain execution
output and assertion events.

Each execution creates a new UUID directory under
`.teshi/logs/replay-screenshots/<run-id>/`. `run.json` is the authoritative,
incrementally published record shared by CLI, desktop GPUI, and web GPUI.
`index.json` remains a compatibility index. Previous runs are never reused to
fill evidence gaps.

## Common contract

`teshi-core::replay_evidence` defines coverage and acceptance. The native
`teshi-replay-evidence` crate owns durable manifests, image validation, history,
and project-scoped artifact access. Backend adapters capture their own targets.

The schema version 2 snapshot exposes `test_status`, `acceptance_status`, and
`evidence` and `evidence_status`. Its compatibility `status` is the acceptance status. A passing test
with missing required evidence is `incomplete`, never an unqualified pass.
Failed assertions retain their original result even if capture also fails.
Historical manifests without an expected step plan remain inspectable with
unknown evidence completeness and unverified acceptance; they are not rewritten.

Coverage counts step executions: `required`, `saved`, `failed`,
`not_executed`, `missing`, and `not_applicable`. `required` equals the sum of
saved, failed, not executed, and missing. One image does not cover a Feature.
Invalid or missing image files are rejected when recording and reopening a run;
TUI coverage additionally requires its SVG and text companions.

Adapters publish `planned_step` before execution, then `start_step`, an
execution outcome, and `step_screenshot`. Events associate Feature, expanded
case/example, step, and `attempt` (default 1). Repeated business executions must
use a new attempt; capture retries must not repeat the business operation.
Capture failures retain the backend error, including codes such as
`executor_busy`, while later test steps remain executable.

Capture metadata includes source, timing, timestamp, file, and image dimensions.
The snapshot records the runner build separately from target identity. Native
launches record the executable content hash when available; unknown target
versions retain an explicit reason. Backend/client code may publish a
`target_identity` event with the application's actual version or deployment ID.

## Teardown and failure

Captures happen after each step, including a failed assertion, before session
cleanup. Explicit close/exit actions capture with `before_close` timing.
For clicks or keystrokes that close their target, set `capture_before: true` on
the confirmed binding's `primary` object, or on the TUI step binding. This is
runner metadata, not an additional Gherkin step.

A failed launch or disconnected session reports capture failure with its
original reason. Planned steps that never execute remain `not_executed`.
GPUI projects missing outcomes with `step_not_executed` or
`capture_outcome_not_recorded`; these rows do not fabricate an execution or an image. The writer finalizes
missing outcomes into the manifest when the run completes or is interrupted.

## External runners

The NDJSON host retains stdout, stderr, assertion events, and the process exit
code in the same run. It supplies `meta.evidence_run_directory` so specialized
adapters write artifacts directly into that run instead of creating duplicates.
The TUI adapter also supports standalone execution with its own managed run.

External visual adapters must emit per-step capture outcomes correlated to the
planned case/step identity. Missing capture integration leaves acceptance
incomplete. Explicit `@cli`, `@logic`, `@unit`, or API-only cases (or
`meta.runner_mode=cli`) are nonvisual; arbitrary UI backends cannot opt out merely
because capture is unsupported. Chrome capture still requires an explicit
target and screenshot capability; the common layer does not select an unrelated
tab or bypass a lease.

The legacy engine protocol receives `screenshot_policy=required` and a managed
`screenshot_dir`. Each `STEP_EXECUTED` must report a fresh `screenshot_path`.
Steps marked `capture_before` must additionally confirm
`capture_timing=before_close`. A returned frame with unconfirmed teardown timing
is retained for inspection but fails evidence acceptance.

Engine attempt IDs default to 1 for older adapters. Report each business retry
with its own `attempt`; reporting self-healing without the corresponding
attempt evidence explicitly fails evidence acceptance.
