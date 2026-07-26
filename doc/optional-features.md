# Optional Feature Flags

## Motivation

The `teshi` binary (`apps/teshi-cli`) unconditionally depends on all frontends:

```
teshi (CLI)
 ├── teshi-daemon  ← always linked (axum, tower-http, uuid, webbrowser, tracing-subscriber...)
 └── teshi-tui     ← always linked (ratatui, crossterm, tui-tree-widget, inquire, arboard...)
```

Even users who only ever run `teshi .` (pure TUI) must compile the entire HTTP daemon dependency chain — axum, tower-http, uuid, tracing-subscriber, webbrowser — none of which are needed for terminal-only usage.

The crate-level separation is already clean — `teshi-core`, `teshi-agent`, and `teshi-engine` have zero UI dependencies. Only the top-level `teshi-cli` binary forces everything together. What's missing is a thin layer of Cargo feature flags to let users opt into only the frontends they need.

## Proposed features

```toml
# apps/teshi-cli/Cargo.toml

[features]
default = ["tui"]
tui = ["dep:teshi-tui"]
web = ["dep:teshi-daemon"]

[dependencies]
teshi-tui = { path = "../../crates/teshi-tui", optional = true }
teshi-daemon = { path = "../../apps/teshi-daemon", optional = true }
```

### Feature matrix

| Feature | Gated dependency | What it enables |
|---|---|---|
| `tui` (default) | `teshi-tui` | Terminal UI — `teshi .`, `teshi auth`, `teshi run`, `teshi record`, `teshi browser` |
| `web` | `teshi-daemon` | Web UI — `teshi web`, `teshi --daemon-internal` |

Desktop (`teshi desktop`) remains a separate binary (`apps/teshi-desktop`) and does not need a feature flag on the CLI crate.

### Conditional compilation in `main.rs`

```rust
fn main() -> anyhow::Result<()> {
    let args = Cli::parse();

    match args.command {
        #[cfg(feature = "web")]
        Some(Subcommand::Web { .. }) => teshi_daemon::run_client(args),

        #[cfg(feature = "tui")]
        _ => teshi_tui::run(args),
    }

    Ok(())
}
```

## Build scenarios

| Use case | Build command | Skipped deps |
|---|---|---|
| TUI only | `cargo build` (default features) | axum, tower-http, uuid, webbrowser, tracing-subscriber |
| Web only | `cargo build --no-default-features -F web` | ratatui, crossterm, tui-tree-widget, inquire, arboard |
| Full build | `cargo build -F tui,web` | nothing (same as today) |

## Compatibility

No breaking changes. The default feature set includes `tui`, preserving the existing `cargo build` behavior for the primary use case. Users who want web support add `-F web`.

The `teshi-daemon` and `teshi-tui` crates themselves do not change — only the `teshi-cli` crate gets feature gates. Their public APIs remain the same.

## Future extensions

- **`desktop` feature** — if the desktop binary is ever merged into the main `teshi` binary, gate it behind `desktop = ["dep:teshi-ui", "dep:gpui"]`.
- **`all` meta-feature** — convenience feature that enables `tui + web` for CI and release builds.
