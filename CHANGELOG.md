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

