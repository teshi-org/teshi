# Explore Run Details demo

A self-contained project for exercising the Explore tab's fourth pane:
**Features | Scenarios / Steps | Run Details**.

The demo runner speaks runner protocol v1 and writes synthetic evidence into the
run directory Teshi creates. It never calls the network and needs no services.

## Prerequisites

- A Teshi build from this branch/worktree. The globally installed `teshi` may
  predate Run Details.
- Python 3.10+ on `PATH` (the runner script is plain standard library).
- Optional: Pillow for labelled screenshots. Without it the runner writes a
  stdlib-generated PNG with the same evidence semantics.

## Run it

From this directory (`demo/explore-run-details`):

```powershell
# Windows PowerShell
cd demo/explore-run-details
..\..\target\debug\teshi.exe .
```

```bash
# Linux / macOS
cd demo/explore-run-details
../../target/debug/teshi .
```

If the binary is missing, build it once from the repository root:

```powershell
cargo build -p teshi-cli
```

The helper scripts `run-demo.ps1` / `run-demo.sh` build when needed and launch
from the correct working directory.

## Try the four content families

The `Login` Feature has four scenarios with different outcomes:

| Scenario | What it demonstrates |
|---|---|
| **Successful login** | `Screenshots` + `API` + `Logs` + `Files`: inline/preview-fallback screenshots, HTTP 200, recording, unknown artifact. Every step also sees the case-level overview screenshot. Select the last step to see the recording. |
| **Rejected password** | Failed step defaults to the `API` tab; `401`, red assertion row, collapsed bodies, screenshot. |
| **Locked account** | `Logs` + `Files` with a recording and an unknown `raw_events` artifact. |
| **Unavailable target** | `BLOCKED` warning chip and category text, with no fabricated evidence. |

`Checkout` has a passed case and a second blocked case so the pane keeps the
same structure when switching Features/runners.

### Steps

1. Launch the demo and stay on the `1 Explore` tab.
2. Select `features/login.feature` in **Features**.
3. Press `r` to run every scenario in that Feature. The demo run takes a few
   seconds and writes real protocol events, screenshots and a tiny MP4.
4. Use `j/k` to select scenarios, then `j/k` in **Steps**; the fourth pane
   follows immediately.
5. Press `Tab` until **Run Details** is focused (its border turns purple), or
   press `l` from Steps.
6. Inside Run Details:
   - `Tab` / `Shift+Tab` cycle the content families.
   - `j/k` select a row; `PageUp`/`PageDown` scroll expanded content.
   - `Space` expands the selected HTTP exchange body.
   - `Enter` opens a screenshot fullscreen, opens a recording, or expands HTTP.
   - `o` hands the selected artifact/URL to the system opener.
   - `y` copies the selected path or URL.
   - `h` returns to Steps.
7. Re-run with `r` at any time; each run gets a fresh evidence directory under
   `.teshi/logs/replay-screenshots/<run>/`.

## Graphics and fallback

The runner always writes PNG files. Whether the pane draws pixels is decided by
the terminal:

```powershell
# Force the no-graphics fallback: same layout, path instead of pixels
$env:TESHI_IMAGE_PROTOCOL='none'
..\..\target\debug\teshi.exe .
```

```powershell
# Sixel terminal (for example Windows Terminal 1.22+)
$env:TESHI_IMAGE_PROTOCOL='sixel'
..\..\target\debug\teshi.exe .
```

Remove the variable to use automatic detection. `halfblocks` is treated as
"off" for UI screenshots by design.

## Narrow terminal

Resize below 70 columns (or under 12 rows). Only the focused pane is drawn and
`Tab` still rotates through Features, Scenarios, Steps and Run Details.

## What the runner emits

`runner/demo_runner.py` emits:

- `start_run` with `steps`, `artifacts`, `blocked`, `http_exchange`
- `start_case` / `start_step` / `end_step`
- `artifact` rows with and without `step_id` (case-level evidence stays
  visible on every step)
- `http_exchange` rows with request/response bodies and assertions
- `log` lines
- `case_passed`, `case_failed`, `case_blocked`, `end_run`

Evidence paths are written inside the run directory under
`.teshi/logs/replay-screenshots/<run>/`, which is the only root Run Details will
open.
