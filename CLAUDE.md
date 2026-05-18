# CLAUDE.md

This project is a Rust application. Follow the rules below when writing, reviewing, or modifying code.

## GitHub English Policy (Always Apply)

This repository is published on GitHub. Use English for all GitHub-facing and collaboration-facing content to keep it accessible for international contributors.

### Must be English
- Git commit messages (subject and body)
- Pull request titles, descriptions, review comments, and merge notes
- Branch names and git tags
- README, CONTRIBUTING, CHANGELOG, release notes, and other top-level docs
- Public code comments and doc comments (e.g. Rust `///` docs)
- User-facing text in source code (CLI help text, error messages, logs intended for users)
- CI/CD workflow names, job names, and visible pipeline messages

### Preferred English
- Source file and directory names
- Identifiers (variables, functions, types, constants)
- Test names and benchmark names

### Allowed Exceptions
- Proper nouns, trademarks, and legal names
- External quotations that must preserve original wording
- Links or references to external Chinese resources (add a short English context line)

### Style Constraints
- Keep wording concise and professional
- Avoid mixed-language sentences in the same line
- Prefer American English spelling for consistency
- If Chinese context is important, write English first and add Chinese in parentheses only when necessary

## Git Commit Message Rules (Always Apply)

- Use a Conventional-Commits style subject line:
  - `feat:` for new features
  - `fix:` for bug fixes
  - `chore:` for maintenance/no user-facing behavior change
  - `docs:` for documentation-only changes
  - `refactor:` for internal restructuring without behavior change
  - `test:` for tests only
- Subject line format:
  - `<type>: <imperative summary>` (imperative mood, no trailing period)
  - Keep it reasonably short (roughly <= 72 characters when possible)
- If the change needs details, add a body after a blank line:
  - Use bullet points for key motivations/decisions and any notable trade-offs
  - Avoid restating the diff; focus on "why" and "what matters"
- If multiple areas changed, consider splitting into multiple commits rather than mixing unrelated concerns.

## Rust Comment and Documentation Policy (Always Apply for `**/*.rs`)

Use comments to explain intent and decisions, not obvious syntax.

### Required Documentation Comments
- Public items MUST include rustdoc comments (`///`): `pub fn`, `pub struct`, `pub enum`, `pub trait`, public methods, and public constants.
- Rustdoc should describe:
  - Purpose and behavior contract
  - Key assumptions/invariants
  - `# Errors` for fallible APIs
  - `# Panics` when panic is possible
  - `# Examples` for non-trivial public APIs

### Required Inline Comments
- Add short comments for complex or non-obvious logic, especially:
  - State transitions and state-machine branches
  - Boundary conditions and off-by-one sensitive code
  - Performance-sensitive trade-offs
  - Safety-critical reasoning and invariants
- Focus comments on "why this approach" and "what must remain true".

### Prohibited Comment Patterns
- Do not add comments that only restate code mechanically.
- Do not leave stale TODO comments without context or owner.
- Do not mix Chinese and English in one comment line.

### Maintenance Rules
- When changing behavior, update related comments/doc comments in the same change.
- If a comment no longer reflects code, fix or remove it immediately.
- Keep comments concise, professional, and in English.

## Rust Coding Style & Idioms (`**/*.rs`)

- Prefer `Result<T, E>` plus the `?` operator for error propagation.
- Avoid `unwrap()`/`expect()` in non-test code; return/handle errors instead.
- Keep functions small; extract helpers and split into modules when it improves clarity.
- Prefer iterator adapters (`map`, `filter`, `fold`, `collect`) over manual loops when readable.
- Borrow wisely: take `&self`/`&T` when possible; clone only when it's necessary and justified.
- Naming: `snake_case` for values/functions/vars, `UpperCamelCase` for types, `SCREAMING_SNAKE_CASE` for consts.
- Run `cargo fmt --all` before finishing changes.

## Rust Error Handling Conventions (`**/*.rs`)

- Use typed errors for libraries; use a general-purpose error (e.g. `anyhow`) for binaries/apps if that matches your project.
- Add context at the boundary where it matters (e.g. when converting/propagating errors to higher layers).
- Prefer domain-specific error types over stringly-typed failures.
- When you define custom errors, derive `std::error::Error` (often via `thiserror`) and implement helpful `Display` messages.
- Don't silence errors with broad `match` + empty arms; handle or propagate them explicitly.
- For long-lived types, consider ownership boundaries carefully (errors should not force unnecessary clones).

## Rust Testing Conventions (`**/*.rs`)

- Prefer fast unit tests for pure logic and deterministic behavior; use integration tests for public API boundaries.
- Put unit tests in the same module behind `#[cfg(test)]` when it helps access private helpers.
- Use `assert_eq!` / `assert!` with clear failure messages (prefer messages over hunting through logs).
- Favor table-driven tests for input/output cases (especially parsing and boundary conditions).
- Name tests descriptively: `test_<scenario>_<expected_behavior>`.
- Include both success and failure paths for error-returning code.
- When debugging, run `cargo test -- --nocapture` to see `println!` output (remove or limit later).

## Rust Documentation (rustdoc) Conventions (`**/*.rs`)

- Document public items with rustdoc comments (`/// ...`): types, public functions, traits, and structs/enums.
- Keep docs focused on intent and usage:
  - What the item does
  - Key invariants/assumptions
  - Panics and error behavior (if any)
- Use a standard section layout when relevant:
  - `# Examples` for runnable usage examples
  - `# Errors` for error types/conditions
  - `# Panics` if the code can panic
- For examples, keep them minimal and compile-ready.
- Avoid copying implementation details; prefer explaining the contract.

## Clippy Linting & Code Quality Guidance (`**/*.rs`)

- Run Clippy regularly: `cargo clippy --all-targets --all-features`.
- Treat new warnings as bugs: prefer fixing the root cause over adding `allow(...)` broadly.
- Prefer idiomatic standard library usage (iterators, conversions, error handling) over manual patterns.
- Common fix targets Clippy often flags:
  - unnecessary borrows/dereferences
  - needless allocations or clones
  - overly complex types when a simpler approach exists
  - functions that return `Ok(...)`/`Some(...)` in redundant ways
- When a lint is false positive or style disagreement, scope the `#[allow(...)]` to the smallest possible item and add a short comment explaining why.

## Cargo Workflow & Quality Checks (`**/*.rs`)

- Use `cargo fmt --all` for formatting.
- Run `cargo clippy --all-targets --all-features` and fix warnings; avoid silencing by default.
- Add/extend tests with `cargo test`; include edge cases for parsing, bounds, and error paths.
- Document public APIs with rustdoc comments (`/// ...`) and keep examples minimal but correct.
- When modifying dependencies/versions, run `cargo update` (if desired) and re-run clippy + tests.

## Cargo Quality Gates (`**/*.rs`)

- Format: `cargo fmt --all`
- Compile check (fast feedback): `cargo check`
- Lint gate (CI-like): `cargo clippy --all-targets --all-features -D warnings`
- Test gate: `cargo test --all`
- Optional docs gate: `cargo doc --no-deps --document-private-items`
