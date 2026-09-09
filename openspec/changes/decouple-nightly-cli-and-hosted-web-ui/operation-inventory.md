# Hosted WebSocket operation inventory

This inventory is the phase-one migration contract. Existing REST routes remain
registered until every hosted UI operation has a control-channel mapping.

| UI area | Current REST route(s) | Planned control method/event | Capability |
| --- | --- | --- | --- |
| LLM config | `/api/v1/llm/config` | `llm.get_config`, `llm.set_config` | `llm_config` |
| Model profiles | `/api/v1/llm/profiles`, `/api/v1/llm/profiles/{id}`, `PUT`, `DELETE`, `.../activate` | `llm.list_profiles`, `llm.get_profile`, `llm.save_profile`, `llm.delete_profile`, `llm.activate_profile` | `llm_config` |
| Browser sessions | `/api/v1/browser/start`, `/api/v1/browser/sessions`, `/api/v1/browser/activate-tab`, `/api/v1/browser/stop` | `browser.start`, `browser.list_sessions`, `browser.activate_tab`, `browser.stop` | `browser_sessions` |
| Project | `/api/v1/projects/open`, `/api/v1/projects/teardown`, `/api/v1/projects/switch-allowed`, `/api/v1/settings/recent`, `/api/v1/settings/project` | `project.open`, `project.teardown`, `project.switch_allowed`, `project.list_recent`, `project.get_settings` | `project` |
| Filesystem | `/api/v1/fs/list`, `/api/v1/fs/read` | `filesystem.list`, `filesystem.read` | `filesystem` |
| Gherkin/BDD | `/api/v1/gherkin/render`, `/api/v1/gherkin/validate-buffer`, `/api/v1/gherkin/scenarios` | `bdd.render_feature`, `bdd.validate_buffer`, `bdd.list_scenarios` | `gherkin` |
| Run/exchange | `/api/v1/daemon/run`, `/api/v1/api/exchange` | `bdd.run`, `api.get_exchange` | `bdd_run`, `api_exchange` |
| Locator/steps | `/api/v1/locator/*`, `/api/v1/steps/*` | `locator.*`, `steps.*` | `locator`, `steps` |
| Terminal | `/api/v1/terminal/spawn`, `/api/v1/terminal/stop`, `/api/v1/terminal/resize`, `/api/v1/terminal/write` | `terminal.spawn`, `terminal.stop`, `terminal.resize`, `terminal.write`; `terminal.output` event | `terminal` |
| Agent/runtime | Runtime events and agent operations used by shared UI | ordered control `event` messages; no standalone Agent RPC is currently used by `apps/teshi-web` | `agent`, `runtime_events` |
| Daemon lifecycle | `/api/v1/daemon/shutdown` | `runtime.shutdown` is reserved and explicitly forbidden to `HostedWebUi`; idle watchdog owns shutdown | `runtime_events` |
| Preview | `/api/v1/browser/stream` | `/ws/preview` frame/error protocol | separate preview channel |

The route list is intentionally retained as a migration reference. The hosted
client must not use the REST routes after its corresponding control method is
implemented and tested.

## Transport limits recorded for phase one

- Control dispatcher concurrency: 16 in-flight requests per authenticated
  connection.
- Control response queue: 64 envelopes; response delivery remains independent
  of the event queue.
- Control event queue: 64 envelopes; terminal/agent bursts are lossy at this
  outer boundary and report a `runtime.overflow` event with a dropped count.
- Hosted WASM client event memory: 256 queued events, oldest first on overflow.
- First-message handshake timeout: 5 seconds.
- Preview sidecar non-frame queue: 8 messages; frames use a newest-frame-wins
  watch slot.
- Preview reconnect grace after control ownership disappears: 5 seconds.
- Local control response isolation budget under a saturated preview/event test:
  100 ms.

These are implementation contracts for the first migration slice. A future
load-test change may tighten the numeric latency threshold without changing the
two-channel architecture.
