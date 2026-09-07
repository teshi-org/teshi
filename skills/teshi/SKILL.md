---
name: teshi
description: Use Teshi to manage requirements, run BDD scenarios, inspect browser or Windows app sessions, and coordinate CLI operations with Teshi Desktop/Web. Use for operating Teshi or choosing its workflow; repository implementation and generic browser automation alone do not require this skill.
---

# Teshi

Choose the Teshi surface that completes the user's task. Use explicit CLI subcommands for agent operations; bare `teshi` enters the interactive TUI.

## Establish context

- Resolve the executable from `TESHI_CLI` when set, otherwise PATH. Check `--version` and the relevant subcommand's `--help`; source-checkout documentation may describe commands absent from the installed release.
- Establish the intended project directory before project-scoped commands. Requirement Markdown is in a separate user-level store; inspect `teshi requirements path` before working on it.
- Prefer structured output where the command supports it. A successful launch or accepted proposal is not evidence that a scenario passed or a binding was confirmed.

## Route the task

| Task | Read or use |
|------|-------------|
| Read/update requirements, run tests, inspect services or export bindings | [CLI workflows](references/cli.md) |
| Open Desktop/Web, explain available panels, coordinate UI and CLI state | [Desktop/Web workflows](references/gui.md) |
| Write or review Gherkin scenarios | `bdd-feature`, if installed |
| Browser target discovery, leases, verified locators and actions | `playwright-locator`, if installed |
| Bind, confirm, or replay an existing Gherkin step | [Binding workflow](references/bindings.md) plus the relevant target specialist |
| Native Windows UIA bindings and regression replay | `winapp-regression`, if installed |

Load only the relevant reference. The packaged `playwright-locator` is the browser-control specialist; this skill owns the Gherkin binding handoff. If a specialist is absent, consult command help and the user's checkout documentation when available; do not assume repository-relative files exist in an installed skill.

Keep the requested action and target explicit. Session discovery, UI selection, and existing connection files provide context, not permission to perform unrelated actions. Reuse authorization already given by the user.
