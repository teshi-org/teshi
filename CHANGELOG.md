## [0.7.11] - 2026-09-21

### Added
#### Acp
- acp: Add protocol-compliant agent client [bae6981](https://github.com/teshi-org/teshi/commit/bae6981)
#### Agent
- Connect acp backend to agent execution [7ac8b26](https://github.com/teshi-org/teshi/commit/7ac8b26)
- agent: Add backend-neutral execution seam [9013914](https://github.com/teshi-org/teshi/commit/9013914)
#### Tui
- tui: Install and select acp agents from registry [2454500](https://github.com/teshi-org/teshi/commit/2454500)
#### Winapp
- winapp: Add real pointer click action [6f48a8a](https://github.com/teshi-org/teshi/commit/6f48a8a)
- winapp: Add lossless element screenshots and pixel asserts [4ad747b](https://github.com/teshi-org/teshi/commit/4ad747b)
- Add negative existence assertions [eafb33f](https://github.com/teshi-org/teshi/commit/eafb33f)
- Connect browser visual observations to agent loop [5bb67eb](https://github.com/teshi-org/teshi/commit/5bb67eb)
- Add production multimodal chat messages [b49e523](https://github.com/teshi-org/teshi/commit/b49e523)
- Establish deepseek v4.1 agent contract [5005d7e](https://github.com/teshi-org/teshi/commit/5005d7e)
- Add gherkin validation and hosted ui transport [a82a060](https://github.com/teshi-org/teshi/commit/a82a060)
- Support github pages hosted web ui [0bc8dd8](https://github.com/teshi-org/teshi/commit/0bc8dd8)
- Decouple nightly cli from hosted web ui [1729068](https://github.com/teshi-org/teshi/commit/1729068)
- Add custom titlebar with window controls for desktop shell [7729c0f](https://github.com/teshi-org/teshi/commit/7729c0f)
- Support proxied github self-updates [a9a9e2b](https://github.com/teshi-org/teshi/commit/a9a9e2b)
- Add windows in-app exe self-update [4b7dfc6](https://github.com/teshi-org/teshi/commit/4b7dfc6)
- Show product version with optional nightly identity [e5f7206](https://github.com/teshi-org/teshi/commit/e5f7206)
- Add teshi requirements cli control plane [cb4361a](https://github.com/teshi-org/teshi/commit/cb4361a)
- Unify teshi agent skills and release packaging [c4a99a1](https://github.com/teshi-org/teshi/commit/c4a99a1)
- Add modal requirements editing and unsaved-change guards [194c26f](https://github.com/teshi-org/teshi/commit/194c26f)
- Add global requirement iterations [5ec9c1a](https://github.com/teshi-org/teshi/commit/5ec9c1a)
- Add gpui run surface and web e2e inspect [1fab966](https://github.com/teshi-org/teshi/commit/1fab966)
- Inspect http exchanges in tui explore [38c0cd5](https://github.com/teshi-org/teshi/commit/38c0cd5)
- Add http api bdd sidecar, dispatch, and teshi api cli [dc8e2e8](https://github.com/teshi-org/teshi/commit/dc8e2e8)
- Show product version in the tui header [a90573a](https://github.com/teshi-org/teshi/commit/a90573a)
- Add install-skill command and ai install runbook [12b265c](https://github.com/teshi-org/teshi/commit/12b265c)
- Add hostname-filtered browser network capture [01e7344](https://github.com/teshi-org/teshi/commit/01e7344)
- Prefer wgc for winapp preview capture [48ed595](https://github.com/teshi-org/teshi/commit/48ed595)
- Expand browser cli control surface [26d9dcb](https://github.com/teshi-org/teshi/commit/26d9dcb)
- Integrate agent browser testing workflows and retire react web ui [67bb4a7](https://github.com/teshi-org/teshi/commit/67bb4a7)
- Add configurable gpui winapp preview [1de6136](https://github.com/teshi-org/teshi/commit/1de6136)
- Unify tui and desktop llm config on shared model profiles (#10) (#10) [5cd21d9](https://github.com/teshi-org/teshi/commit/5cd21d9)
- Add nightly pre-release builds from dev branch (#7) (#7) [954692b](https://github.com/teshi-org/teshi/commit/954692b)

### Changed
#### Agent
- Extract native agent runtime from tui [0297a03](https://github.com/teshi-org/teshi/commit/0297a03)
- Add openspec change for http api bdd testing [aae68a4](https://github.com/teshi-org/teshi/commit/aae68a4)
- Document http api bdd conventions and teshi api cli [6b791fa](https://github.com/teshi-org/teshi/commit/6b791fa)
- Update readme development section and refresh development guide [75ad318](https://github.com/teshi-org/teshi/commit/75ad318)
- Consolidate agent skills into three task workflows [c8bf0fa](https://github.com/teshi-org/teshi/commit/c8bf0fa)

### Fixed
#### Agent
- agent: Restore turns after cancellation and validate replay first [19668fb](https://github.com/teshi-org/teshi/commit/19668fb)
#### Ci
- ci: Copy windows runtime with safe powershell path [351ab9e](https://github.com/teshi-org/teshi/commit/351ab9e)
#### Tui
- tui: Require ctrl-w for requirements pane switching [441a647](https://github.com/teshi-org/teshi/commit/441a647)
- Shorten nightly version display [2e01786](https://github.com/teshi-org/teshi/commit/2e01786)
- Make update checks observable and bounded [b28859f](https://github.com/teshi-org/teshi/commit/b28859f)
- Avoid non-windows updater dead code warning [aed4e0a](https://github.com/teshi-org/teshi/commit/aed4e0a)
- Preserve hyphenated runner arguments [3f5c51d](https://github.com/teshi-org/teshi/commit/3f5c51d)
- Satisfy ci formatting and clippy gates [b6a8262](https://github.com/teshi-org/teshi/commit/b6a8262)
- Harden deepseek v4.1 agent contract [1e24c88](https://github.com/teshi-org/teshi/commit/1e24c88)
- Dispatch slash menu commands [2d94f84](https://github.com/teshi-org/teshi/commit/2d94f84)
- Sync requirements tree selection [b5d5aeb](https://github.com/teshi-org/teshi/commit/b5d5aeb)
- Use hosted origin allowlist in daemon [d2ac3bc](https://github.com/teshi-org/teshi/commit/d2ac3bc)
- Make nightly packaging cli-only [bc9b34a](https://github.com/teshi-org/teshi/commit/bc9b34a)
- Bypass proxies for local web daemon bootstrap [1399980](https://github.com/teshi-org/teshi/commit/1399980)
- Reject common credential files from hosted ui [0cd19b7](https://github.com/teshi-org/teshi/commit/0cd19b7)
- Gate windows-only update tls helpers for unix clippy [206af30](https://github.com/teshi-org/teshi/commit/206af30)
- Drop needless return in unix setup stub [9c2bf6a](https://github.com/teshi-org/teshi/commit/9c2bf6a)
- Make update and screenshot tests pass on unix ci [2e53725](https://github.com/teshi-org/teshi/commit/2e53725)
- Keep release checksum verification valid yaml [df24a57](https://github.com/teshi-org/teshi/commit/df24a57)
- Load requirement store when teshi starts without a path [9ce715e](https://github.com/teshi-org/teshi/commit/9ce715e)
- Skip nightly tags for unchanged commits (#15) (#15) [87f4bf5](https://github.com/teshi-org/teshi/commit/87f4bf5)
- Secure daemon access boundaries [1fd60fe](https://github.com/teshi-org/teshi/commit/1fd60fe)
- Skip nightly tags for unchanged commits [ee1c84f](https://github.com/teshi-org/teshi/commit/ee1c84f)
- Pass commit sha as release target_commitish (#13) (#13) [db37329](https://github.com/teshi-org/teshi/commit/db37329)
- Ignore nightly tags when computing last release [66e1962](https://github.com/teshi-org/teshi/commit/66e1962)
- Use slice fill for screen dirty flags [5bc210e](https://github.com/teshi-org/teshi/commit/5bc210e)
- Run browser-agent package smoke test without a local debug cli [e5d7a24](https://github.com/teshi-org/teshi/commit/e5d7a24)
- Include rule-nested scenarios in explore and mindmap [afd6343](https://github.com/teshi-org/teshi/commit/afd6343)
- Align tui tab hit-testing with rendered layout [c4b80b2](https://github.com/teshi-org/teshi/commit/c4b80b2)
- Harden nightly reusable workflow and resolve metadata (#8) (#8) [f79b8fe](https://github.com/teshi-org/teshi/commit/f79b8fe)

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

