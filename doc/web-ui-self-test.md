# GPUI WASM web UI self-test

`teshi web` serves the GPUI WASM shell from `apps/teshi-web/dist`. The retired
React/Vite frontend has been removed; GPUI WASM is the only runtime, release
artifact, and frontend quality gate.

## Build and run

From the repository root:

```bash
bash scripts/build-teshi-web.sh
cargo build -p teshi-cli
./target/debug/teshi web --project . --host 127.0.0.1 --port 20253 --no-open
```

On Windows, use `scripts/build-teshi-web.ps1` and `target\debug\teshi.exe`.
The daemon auto-resolves `apps/teshi-web/dist` from a source checkout. Installed
packages resolve the bundled `share/web` directory.

## Automated smoke gate

```bash
bash scripts/run-web-ui-smoke.sh
```

The gate builds the wasm32 GPUI target, runs `wasm-bindgen`, verifies the runtime
marker and non-empty `.wasm` output, and rejects React/Vite runtime markers.

## Browser-agent validation

1. Load the Teshi Bridge extension in one or more Chrome profiles.
2. Start the Chrome bridge with `teshi browser start --mode chrome`.
3. Open `http://127.0.0.1:20253/` in a leased target tab.
4. Confirm the page title is `teshi — GPUI Web` and the page displays the GPUI
   Browser Profiles canvas.
5. With multiple profiles connected, confirm no profile is selected automatically.
   Select a profile explicitly before inspecting or activating one of its tabs.

Useful checks:

```bash
curl --noproxy '*' http://127.0.0.1:20253/
curl --noproxy '*' http://127.0.0.1:20253/api/v1/browser/sessions
teshi browser sessions
```

The HTML response must contain
`<meta name="teshi-ui-runtime" content="gpui-wasm">`. Browser operations with
several live profiles must include the session/window/tab target and a valid lease;
ambiguous operations fail without mutating browser state.
