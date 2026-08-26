# teshi

Terminal-first BDD editor with AI assistance, mind-map navigation, and external test runner integration.

```
teshi .              # open current project
teshi web            # browser GUI
teshi desktop        # native desktop (WinUI3 recording)
```

Explore · MindMap · AI — three tabs for browsing, editing, and AI-assisted authoring.

---

**Install** — `winget install teshi-org.teshi` · [releases](https://github.com/teshi-org/teshi/releases)

**Next steps** — [User Guide](doc/user-guide.md) · [Installation](doc/installation.md) · [CLI & Config](doc/cli-usage.md) · [Keybindings](doc/keybindings.md) · [Development](doc/development.md)

---

## Development

To build and run `teshi` from source:

```bash
# 1. Clone the repository
git clone https://github.com/teshi-org/teshi.git
cd teshi

# 2. Install the Rust toolchain (skip if you already have rustup)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 3. Build the project
cargo build --release

# 4. Run tests
cargo test

# 5. Run the CLI
cargo run -- .
```

See the full [Development Guide](doc/development.md) for project structure, code conventions, release workflow, and platform-specific notes.

**License** MIT
