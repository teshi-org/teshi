# Desktop and Web workflows

## Launch the intended surface

```text
teshi desktop --project path/to/project
teshi web --project path/to/project --no-open
```

Desktop is the native GPUI shell; Web serves the GPUI WASM shell through the daemon, normally at `127.0.0.1:20253`. Check installed help and the actual startup output for the address and available flags.

Web requires built assets. Windows full MSI/release archives bundle them under `share/web` beside the executable. In a source checkout, `bash scripts/build-teshi-web.sh` builds `apps/teshi-web/dist`; use `teshi web --dist apps/teshi-web/dist --project <path> --no-open` when needed. The build requires nightly Rust, the WASM target, and wasm-bindgen. A missing dist error calls for obtaining/building assets, not starting a Vite server.

## Current capability map

The shared GPUI shell has these surfaces. Availability of target operations still depends on the platform, connection, and reported capabilities.

| Surface | User-visible purpose | Agent coordination |
|---------|----------------------|--------------------|
| Browser | Discover/select browser profiles and inspect their windows/tabs | Rediscover the explicit target through CLI; UI selection alone does not supply an exclusive lease. |
| WinApp | Native Windows application screenshot stream | Check attachment and frame freshness; use `winapp` for supported native operations. |
| Run | List/run scenarios and inspect events and HTTP exchanges | Use `run`/`api` for scripted work; inspect final outcomes. This panel is a read-only event inspector, not a Gherkin editor. |
| Settings | LLM configuration | Explain the relevant visible setting or use an actually supported interface; avoid inventing CLI settings commands. |

Gherkin editing, requirement authoring, and the Explore/MindMap/AI workflows belong to the TUI. Use explicit `requirements` commands for supported non-interactive library work. Do not promise a GPUI requirement editor, embedded terminal, or step-selection panel based on older Teshi frontend documentation.

## UI and CLI handoffs

Establish the same project on both sides. When a workflow uses `.teshi/active-step.json`, inspect whether it exists and matches the intended feature/step; do not assume the current GPUI shell creates it. `.teshi/cdp-endpoint.json` describes connection context, not proof of liveness or target ownership. Inspect current sessions/health before actions.

For a user request to bind a step, follow [Gherkin bindings and replay](bindings.md). `steps select --feature <path> --line <N>` selects recording context without a GUI selector. If the installed surface does not offer the expected confirmation control, use supported CLI options consistent with the user's requested review mode rather than inventing a button.

When the user requests a visual check, inspect the actual rendered panel and relevant result. CLI success alone does not verify UI presentation. Conversely, opening a panel alone does not verify that a scenario or binding succeeded.

## Testing the Teshi UI itself

Teshi Web is GPUI rendered in-browser; do not assume its visual widgets are ordinary HTML elements. In a source checkout, consult `doc/web-ui-self-test.md` for the supported test bridge and evidence. `scripts/run-web-ui-smoke.sh` checks the built distribution; it does not prove every user workflow. Use the actual configured test URL and selectors, never old React/Vite port or DOM assumptions.
