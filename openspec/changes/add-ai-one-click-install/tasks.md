## 1. CLI command

- [x] 1.1 Add `teshi install-skill` (`--dry-run`, `--yes`) to `crates/teshi-tui/src/cli/mod.rs` and dispatch it from `lib.rs`
- [x] 1.2 Implement source resolution, plan printing, copy into `~/.agents/skills/<name>`, discovery-path symlinks, skip-real-dir, and Windows symlink error hints in `install_skill.rs`

## 2. Tests

- [x] 2.1 Cover dry-run (no writes), share and checkout source resolution, copy + symlink, missing-parent skip, and non-symlink directory skip
- [x] 2.2 Cover packaged-over-repo name collision and missing-source error text that does not point at GitHub Skill downloads

## 3. Docs

- [x] 3.1 Add English `AI_INSTALL.md` with the CLI / teshi-bridge / dry-run then install-skill sequence
- [x] 3.2 Add README / README_zh / installation.md / browser-modes.md one-line pointers to `AI_INSTALL.md` and `teshi install-skill`

## 4. Verification

- [x] 4.1 Run `cargo fmt --all` and `cargo test -p teshi-tui --locked`
