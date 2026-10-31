# TUI screenshot viewing

Captures are reached from where a scenario is run and reviewed. In Explore the
Scenarios and Steps panes end rows with `●N`, and `i` opens the capture of the
selected step. The Failure Details popup opens the failing step's capture with
`i` from any Explore pane, and closing the viewer moves the Steps selection to
the capture that was last shown. `i` on a MindMap node opens the same viewer for
browsing evidence by requirement structure. The viewer uses the newest replay
run with captures for that scenario.

In Explore the viewer covers the Scenarios and Steps panes and leaves the
Features column visible; below 84 columns of tab width it covers the whole tab.
On the MindMap tab it always covers the tab.

| Key | Behavior |
| --- | --- |
| `j` / `k` | Next / previous captured step |
| `+` / `=` / `-` | Increase / decrease magnification |
| `1` | View original pixels at 100 percent |
| `0` / `f` | Fit the image and reset its center |
| Arrow keys | Pan a zoomed view by a quarter viewport |
| `h` / `l` | Pan horizontally |
| Up / Down in fit view | Previous / next captured step |
| `o` | Open the original in the operating system's viewer |
| `i` / Esc | Close immediately, including during loading |

100 percent is measured in the terminal graphics protocol's virtual pixels;
the terminal may scale those pixels for its font and display DPI. Use the
system viewer for physical-pixel inspection outside the terminal.

The title shows source dimensions, magnification, and source coordinates for
zoomed views. Loading means a background task is reading, decoding, resizing,
or encoding. Drawing means encoded Sixel/iTerm2 tiles are still being sent.
Resizing the terminal preserves magnification and the normalized source center.
Zooming reads a source region from the decoded original, rather than magnifying
an already reduced preview.

## Responsiveness and resource limits

One worker processes the latest requested view. A newer request replaces the
pending request; generation checks discard obsolete completions. Closing the
viewer and quitting do not wait for an active decoder. Decoders cannot be
interrupted inside a single library call; cancellation is checked between file
I/O, decoding, resizing, and individual encoding tiles.

| Resource | Limit |
| --- | --- |
| Compressed capture file | 64 MiB |
| Decoded image | 128 MiB and 32 megapixels (33,554,432 pixels) |
| Decoded-image LRU cache | 128 MiB |
| Encoded-view LRU cache | 32 MiB |
| Output canvas | 2,097,152 virtual pixels |
| Encoded Sixel/iTerm2 view | 8 MiB |
| Sixel/iTerm2 output per UI iteration | 64 KiB, including cursor commands |

These are per-resource limits, not a total process-memory ceiling. A decoder,
source-region copy, resampling buffers, a newly encoded view, and existing cache
entries may coexist. An output canvas larger than the limit is reduced in cell
dimensions and centered in the panel. Oversized inputs are rejected with an
external-viewer hint before full pixel decoding. Cache identity includes the
source path, file length and modification time, view transform, viewport, and
protocol. The source is reloaded when that identity changes.

After the current view is published, the worker prepares the fit view of the
next and previous captures into the encoded-view cache, so `j` / `k` usually
show a cached view. Any new request cancels this prefetch at the next phase
or tile boundary. Prefetch does not keep decoded sources, so it cannot evict
the source that zooming the current capture needs.

Only the cells the image covers are encoded; letterbox space stays terminal
background. Sixel tiles use `icy_sixel`'s Wu quantizer with 256 colors per
tile and no dithering, which keeps UI text sharp. Tile rows are encoded on a
few scoped threads. The workspace compiles the image, PNG, JPEG, and Sixel
crates with `opt-level = 3` in the dev profile, because unoptimized decoding
and quantization are 10 to 30 times slower and local installs use that
profile.

Sixel and iTerm2 tiles are encoded on the worker. The UI first clears the
image area as ordinary text, then sends a bounded batch of tiles after the
frame flush. Input is processed between batches, and the event loop waits at
most a few milliseconds per iteration while a view is loading or drawing.
Idle frames do not resend an image. Replacing or closing a capture, or resizing the terminal, clears stale
pixel output before repainting. Transport throughput can still limit a single
bounded write; this is not a hard real-time latency guarantee.

## Terminal compatibility

Automatic detection requires actual protocol support. `TESHI_IMAGE_PROTOCOL`
is a diagnostic override, not a way to make a terminal support images. If no
protocol is available, the viewer shows the reason and the path to the capture.
It does not silently substitute a low-resolution half-block preview.

The Windows Zellij/system-ConPTY path must be tested separately from a direct
Windows Terminal session or a PTY using packaged `conpty.dll` / `OpenConsole.exe`.
Some native console runtimes filter Sixel before it reaches the multiplexer.
The application cannot recover pixels removed by that runtime. A forced
protocol in Windows Zellij displays a compatibility hint even while preparing
a view.

## Validation

Unit tests cover source-pixel cropping, pan bounds, resource limits, bounded
LRU eviction, source-file invalidation, latest-request coalescing, discarded
late results, prompt worker drop, text clearing, and bounded tile output.

The paired `tui_image_viewer.feature` / `流畅截图查看.feature` files are bound
by `teshi-image-feature-runner` and
`crates/teshi-tui-runner/tests/fixtures/image_feature_steps.py`.
The runner parses the exact source steps, rejects unknown steps, creates an
isolated project with 8192 by 4096 and 1024 by 512 captures, and drives the
actual CLI through an owned PTY. It records per-step pass/fail events, viewport
text, SVGs, PNG previews, executable hashes, and timing measurements. Each
Background starts the tested application with its isolated captures so fixture
setup also has a real product viewport. Exit actions capture before closing;
subsequent exit assertions observe the retained PTY viewport. Timing boundaries
exclude evidence rasterization between source steps. Header-only oversized
PNG fixtures exercise decoder rejection without allocating the declared pixels.

```powershell
cargo build -p teshi-cli --locked
cargo build --manifest-path crates/teshi-tui-runner/Cargo.toml --bin teshi-image-feature-runner --locked
$env:TESHI_SELF_TEST_BIN = (Resolve-Path target/debug/teshi.exe).Path
$runner = (Resolve-Path crates/teshi-tui-runner/target/debug/teshi-image-feature-runner.exe).Path
& $env:TESHI_SELF_TEST_BIN check --feature features/en-US/tui_image_viewer.feature --json
& $env:TESHI_SELF_TEST_BIN check --feature features/zh-CN/流畅截图查看.feature --json
& $env:TESHI_SELF_TEST_BIN run features/en-US/tui_image_viewer.feature --runner-cmd $runner --runner-cwd (Get-Location).Path
& $env:TESHI_SELF_TEST_BIN run features/zh-CN/流畅截图查看.feature --runner-cmd $runner --runner-cwd (Get-Location).Path
```

These PTY assertions prove controls, cancellation, source-region metadata and
process lifecycle. A text or SVG viewport does not establish terminal pixel
visibility. Direct-terminal and Windows Zellij visual acceptance remain distinct.
