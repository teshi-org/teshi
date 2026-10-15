# Verification

Verified on Windows from `D:\Dev\Rust\teshi\dev` with an isolated `CARGO_TARGET_DIR=target\acp-cli-verify` after the shared Cargo cache was temporarily locked by concurrent invocations.

- `cargo fmt --all --check`: passed.
- `cargo test -p teshi-acp --locked -j 1`: 30 passed across unit and client behavior tests; 2 existing real CodeBuddy tests ignored because they require the local external package.
- `cargo test -p teshi-tui cli::acp --locked -j 1`: 3 passed.
- `cargo test -p teshi-tui acp_agents --locked -j 1`: 5 passed.
- `cargo check --workspace --exclude teshi-web --locked -j 1`: passed.
- `cargo run -p teshi-cli --locked -- acp --help`: passed and listed `list`, `show`, `add`, `remove`, and `config-path`.
- `cargo run -p teshi-cli --locked -- acp config-path`: passed and printed the resolved absolute user configuration path.
- `openspec validate add-custom-acp-cli-config --strict`: passed.
- `git diff --check`: passed.

The smoke run did not add or remove a real user ACP definition. Mutation behavior is covered through temporary-directory unit tests, including failed-write preservation.
