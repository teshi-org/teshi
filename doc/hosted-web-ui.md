# Hosted GPUI Web UI

The production browser UI is the latest GPUI WASM bundle published at
`https://teshi-org.github.io/app/`. The custom domain
`https://teshi.org/app/` remains a compatible alternate entrypoint. The nightly
CLI does not contain or serve a complete Web UI.

## Launch lifecycle

`teshi web` starts (or reuses) the project daemon on `127.0.0.1`. When it starts
a daemon without `--port`, the child binds port `0`, records the OS-selected
port in `.teshi/daemon.json`, and the CLI mints an in-memory `HostedWebUi`
session. It then opens:

```text
https://teshi-org.github.io/app/#port=<port>&token=<session-token>
```

The token is never logged or written to disk. The hosted page reads the
fragment into memory and removes it with `history.replaceState` before normal
startup. The fragment is therefore not sent to the Pages host.

Each new `teshi web` launch replaces any previous hosted launch token. An
explicit project/runtime teardown invalidates hosted tokens while preserving
unrelated local automation sessions; an already-open socket rejects its next
request and closes.

The page negotiates a versioned `ws://127.0.0.1:<port>/ws/control` connection
for RPC and ordered runtime events. Browser and WinApp frames use a separate
`/ws/preview` connection. Both sockets require one of the exact trusted
Origins `https://teshi.org` or `https://teshi-org.github.io`, plus a
first-message session/protocol handshake.
For a local hosted-page harness only, a debug/test daemon may set
`TESHI_DEV_WEB_ORIGIN` to one explicit origin; release binaries ignore this
override and never use a wildcard.

Hosted browser-session RPCs expose only the allowlisted extension/session
metadata. Sidecar WebSocket URLs, CDP endpoint paths, project roots, and
broker command tokens remain daemon-private and are never sent to the hosted
page or runtime event stream. The `HostedWebUi` token is not authorized for
legacy `/api/v1/*` routes; hosted business traffic uses `/ws/control` and
`/ws/preview` only. Hosted filesystem access is project-relative and hides
`.teshi`, VCS metadata, and credential-like files. HTTP exchange inspection is
always redacted for the hosted page, including URL credentials/query strings
and unstructured request/response bodies.

## Local development checks

Build and inspect the WASM artifact without changing the production launcher:

```bash
bash scripts/run-web-ui-smoke.sh
python scripts/test-hosted-ui-transport.py
```

The `--dist` option remains only for explicit daemon/static-file diagnostics
and is not required by the production `teshi web` path. The supported browser
acceptance test must use a deployed Pages artifact and a real daemon session;
an arbitrary `http://127.0.0.1` page is not a substitute because production
Origin validation is intentionally exact.

## Compatibility and migration

`/app/ui-manifest.json` declares the minimum nightly build sequence and control
and preview protocol versions. An older CLI fails closed with an upgrade
message; it does not select a historical UI. Existing `/api/v1/*` routes remain
available during this first migration phase and are removed only by a separate
reviewed change after hosted UI coverage is proven.
