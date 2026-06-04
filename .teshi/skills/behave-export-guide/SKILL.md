---
name: behave-export-guide
description: Configure and run a teshi-exported behave project for WinUI3 UIA tests without teshi in CI
---

# Behave Export Guide

Use after `teshi export --target behave` generated a `tests-e2e/` (or custom) directory.

## Layout

```text
tests-e2e/
  features/           # copied .feature files
  pages/              # Page objects (AutomationId constants)
  steps/              # @given/@when/@then (Chinese step text as decorators)
  support/
    environment.py    # launches APP_EXE via uiautomation
  .env.example
  requirements.txt
  README.md
```

## Setup (Agent-guided, one time)

1. Copy `.env.example` to `.env`.
2. Set `APP_EXE` to the built WinUI3 application path.
3. Optional: `TEST_PASSWORD`, `LAUNCH_TIMEOUT_MS`, etc.
4. Install deps:

```bash
cd tests-e2e
python -m venv .venv
.venv\Scripts\activate   # Windows
pip install -r requirements.txt
```

## Run locally

```bash
behave features/
behave features/your.feature -n "Scenario name"
```

## CI

- Do **not** require teshi Desktop or CLI.
- Install Python + app artifact + `pip install -r requirements.txt`.
- Set `APP_EXE` (and secrets) as pipeline variables.
- Run `behave --junit` (or your reporter).

## Updating after UI changes

1. Re-bind changed steps in teshi (winapp-locator).
2. Re-run `teshi export --target behave ...` (overwrites generated files).
3. Commit updated `features/`, `pages/`, `steps/`, and project `.teshi/step-bindings/`.

## Do not

- Hand-edit generated files without re-exporting (files are marked generated).
- Expect behave to call LLMs or teshi at runtime.
