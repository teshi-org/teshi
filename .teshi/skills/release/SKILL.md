---
name: release
description: Manage teshi releases — analyze commits, bump versions, tag, and publish
---

# Release Skill

This skill helps prepare and execute teshi releases. It wraps the project's `scripts/release.py` tool.

## When to use

- User says "发布新版本", "发版", "release", "bump version", "publish a new release"
- User wants to know what the next version should be
- Version files are out of sync and need alignment

## How to use

The release workflow is handled by `scripts/release.py` at the project root. No external dependencies beyond Python 3.8+ and git.

### Workflow

1. **Dry-run first** to see what version is recommended:
   ```
   python scripts/release.py
   ```
   This analyzes commits since the last tag and shows the recommended version bump (major/minor/patch).

2. **Apply version bumps** to all 5 component files:
   ```
   python scripts/release.py --apply
   ```

3. **Full automation** — apply, commit, and tag:
   ```
   python scripts/release.py --tag
   ```

4. **Push** to trigger CI release:
   ```
   git push origin main && git push origin vX.Y.Z
   ```

### Key details

- All 5 version files must stay in sync: `Cargo.toml`, `desktop/src-tauri/Cargo.toml`, `desktop/package.json`, `desktop/src-tauri/tauri.conf.json`, `extension/teshi-bridge/manifest.json`
- Version bump is determined from conventional commits since the last tag
- The pre-commit hook validates version consistency across all 5 files
- CI (GitHub Actions) auto-builds and publishes when a `v*` tag is pushed
