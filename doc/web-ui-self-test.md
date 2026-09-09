# GPUI WASM hosted Web UI self-test

`teshi web` launches the local daemon and opens the latest GPUI WASM shell from
`https://teshi.org/app/`. The retired React/Vite frontend has been removed.
Nightly installers do not contain `share/web`.

## Build and run

From the repository root:

```bash
cargo build -p teshi-cli
./target/debug/teshi web --project .
```

On Windows, use `target\debug\teshi.exe`. The default launch chooses an OS
available loopback port and creates a process-memory-only session.

## Automated smoke gate

```bash
bash scripts/run-web-ui-smoke.sh
```

The gate builds the wasm32 GPUI target, runs `wasm-bindgen`, verifies the runtime
marker and non-empty `.wasm` output, rejects React/Vite runtime markers, and
checks the control/preview transport contract. It does not publish or package
the result into the nightly CLI.

## Browser-agent validation

1. Load the Teshi Bridge extension in one or more Chrome profiles.
2. Start the Chrome bridge with `teshi browser start --mode chrome`.
3. Run `teshi web` and let it open the deployed `https://teshi.org/app/` page.
4. Confirm the page title is `teshi — GPUI Web` and the page displays the GPUI
   Browser Profiles canvas.
5. With multiple profiles connected, confirm no profile is selected automatically.
   Select a profile explicitly before inspecting or activating one of its tabs.

The production page must connect to the daemon only through `/ws/control` and
`/ws/preview`; use the browser/network panel or the supported Chromium smoke
suite to verify this. Existing `/api/v1/*` endpoints remain for migration and
legacy clients, but are not the hosted UI transport.

The HTML response must contain
`<meta name="teshi-ui-runtime" content="gpui-wasm">`. Browser operations with
several live profiles must include the session/window/tab target and a valid lease;
ambiguous operations fail without mutating browser state.
