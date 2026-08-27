## [0.7.10] - 2026-08-27

### Added
- Add install-skill command and ai install runbook [12b265c](https://github.com/teshi-org/teshi/commit/12b265c)
- Add hostname-filtered browser network capture [01e7344](https://github.com/teshi-org/teshi/commit/01e7344)
- Prefer wgc for winapp preview capture [48ed595](https://github.com/teshi-org/teshi/commit/48ed595)
- Expand browser cli control surface [26d9dcb](https://github.com/teshi-org/teshi/commit/26d9dcb)
- Integrate agent browser testing workflows and retire react web ui [67bb4a7](https://github.com/teshi-org/teshi/commit/67bb4a7)
- Add configurable gpui winapp preview [1de6136](https://github.com/teshi-org/teshi/commit/1de6136)
- Unify tui and desktop llm config on shared model profiles (#10) (#10) [5cd21d9](https://github.com/teshi-org/teshi/commit/5cd21d9)
- Add nightly pre-release builds from dev branch (#7) (#7) [954692b](https://github.com/teshi-org/teshi/commit/954692b)
- Route requirements generation through tui and drop daemon api [3b619c1](https://github.com/teshi-org/teshi/commit/3b619c1)
- Improve teshi-tui windows input and mouse capture [a4cab22](https://github.com/teshi-org/teshi/commit/a4cab22)
- Surface teshi-web gpu startup errors in the loading ui [80486a3](https://github.com/teshi-org/teshi/commit/80486a3)
- Show wasm download progress bar on teshi-web load [c0bc83f](https://github.com/teshi-org/teshi/commit/c0bc83f)
- Add multi-profile llm config across engine, ui, and daemon [b36ebc5](https://github.com/teshi-org/teshi/commit/b36ebc5)
- Move llm config into shared appshell settings [f7e67e1](https://github.com/teshi-org/teshi/commit/f7e67e1)
- Add shared gpui llm config spike for desktop and web [d567d55](https://github.com/teshi-org/teshi/commit/d567d55)
- Requirements-to-testpoints page implementation [7e37eef](https://github.com/teshi-org/teshi/commit/7e37eef)
- Add teshi terminal cli with vte screen grid sidecar [8f1e714](https://github.com/teshi-org/teshi/commit/8f1e714)

### Changed
- Update readme development section and refresh development guide [75ad318](https://github.com/teshi-org/teshi/commit/75ad318)
- Consolidate agent skills into three task workflows [c8bf0fa](https://github.com/teshi-org/teshi/commit/c8bf0fa)
- Refresh cursor cloud setup notes for current workspace layout (#4) (#4) [76fd8ba](https://github.com/teshi-org/teshi/commit/76fd8ba)
- Add agents.md with cursor cloud setup notes (#3) (#3) [868d43a](https://github.com/teshi-org/teshi/commit/868d43a)
- Document optional cli feature flags proposal [c05a89d](https://github.com/teshi-org/teshi/commit/c05a89d)
- Replace teshi-tauri web shell with apps/teshi-web-ui [a81c421](https://github.com/teshi-org/teshi/commit/a81c421)
- Move browser/winapp service scripts to resources/ [334f1c2](https://github.com/teshi-org/teshi/commit/334f1c2)
- Sync openspec for retiring freemind requirements page [cca09dc](https://github.com/teshi-org/teshi/commit/cca09dc)
- Split monolith into layered crates and app shells [2b5e9f8](https://github.com/teshi-org/teshi/commit/2b5e9f8)

### Fixed
- Ignore nightly tags when computing last release [41dca1b](https://github.com/teshi-org/teshi/commit/41dca1b)
- Use slice fill for screen dirty flags [5bc210e](https://github.com/teshi-org/teshi/commit/5bc210e)
- Run browser-agent package smoke test without a local debug cli [e5d7a24](https://github.com/teshi-org/teshi/commit/e5d7a24)
- Include rule-nested scenarios in explore and mindmap [afd6343](https://github.com/teshi-org/teshi/commit/afd6343)
- Align tui tab hit-testing with rendered layout [c4b80b2](https://github.com/teshi-org/teshi/commit/c4b80b2)
- Harden nightly reusable workflow and resolve metadata (#8) (#8) [f79b8fe](https://github.com/teshi-org/teshi/commit/f79b8fe)
- Enforce test point traceability gates (#6) (#6) [dc6c942](https://github.com/teshi-org/teshi/commit/dc6c942)

## [0.7.9] - 2026-06-26

### Added
- Add --host flag and change default port from 1421 to 20253 [125c9bd](https://github.com/teshi-org/teshi/commit/125c9bd)
- Fix terminal continuous scroll [608645e](https://github.com/teshi-org/teshi/commit/608645e)
- Add bdd-feature-convention skill [e83c3f9](https://github.com/teshi-org/teshi/commit/e83c3f9)
- Add browser agent exploration tools and trace cli [02f94fb](https://github.com/teshi-org/teshi/commit/02f94fb)
- Expose stepindex via daemon api + cli catalog command [0122185](https://github.com/teshi-org/teshi/commit/0122185)
- Add session-based auth with role permissions [807269f](https://github.com/teshi-org/teshi/commit/807269f)
- Support specifying port for daemon web server [f6b25b7](https://github.com/teshi-org/teshi/commit/f6b25b7)
- Auto-generate changelog.md on release [ee33b78](https://github.com/teshi-org/teshi/commit/ee33b78)

### Changed
- Rename to skill.md and optimize description [4c15215](https://github.com/teshi-org/teshi/commit/4c15215)
- Remove custom tskill template system [2fe84e5](https://github.com/teshi-org/teshi/commit/2fe84e5)

### Fixed
- Deduplicate terminal output via websocket connection fix [3e24309](https://github.com/teshi-org/teshi/commit/3e24309)
- Make gherkin step parser skip continuation lines instead of breaking [2a45aee](https://github.com/teshi-org/teshi/commit/2a45aee)
- Keep psreadline loaded to prevent embedded terminal auto-enter [63f4832](https://github.com/teshi-org/teshi/commit/63f4832)

# Changelog

All notable changes to this project will be documented in this file.

