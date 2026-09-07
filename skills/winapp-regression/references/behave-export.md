# Behave export

Use after `teshi export --target behave` generated a `tests-e2e/` (or custom) directory. Inspect the generated README, requirements, and environment module for the installed exporter's actual layout and supported actions.

## Layout

```text
tests-e2e/
  behave.ini              # paths = features
  features/
    *.feature             # copied feature files
    environment.py        # UiaDriver + app launch
    steps/
      *_steps.py          # @given/@when/@then (step text as decorators)
  pages/
    *_page.py             # Page objects (AutomationId constants)
  support/                # optional legacy; prefer features/environment.py
  .env.example
  requirements.txt
  README.md
```

Run from `tests-e2e/` with no manual junctions:

```bash
behave
behave features/your.feature -n "Scenario name"
```

## Setup (one time)

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
behave --dry-run          # validate step defs without launching app
behave
```

Inspect both dry-run output and exit status. A dry-run validates definitions but does not prove UI behavior.

## Feature naming (non-ASCII)

Chinese or other non-ASCII feature file names produce a safe `page_module` (e.g. `u5e93_u754c...` or `feature_<hash>`). Prefer English feature file names when you want readable Python module names.

## Selectors in exported tests

`UiaDriver` supports teshi binding formats:

- `uia:automation_id=X`
- `uia:name=X`
- `uia:control_type=ButtonControl;name=Log in`
- `uia:path=0/2/1` (last resort)

`assert_text` uses **exact** UIA `Name` match, not substring. Use `exec` bindings or custom helpers for partial text.

## CI

- Exported tests run independently of Teshi Desktop/CLI; Windows UIA still requires an appropriate interactive Windows session and the application under test.
- Install Python + app artifact + `pip install -r requirements.txt`.
- Set `APP_EXE` (and secrets) as pipeline variables.
- Run `behave --junit` (or your reporter).

## Updating after UI changes

1. Re-bind changed steps in teshi (`steps unbind` wrong bindings first).
2. Re-run `teshi export --target behave ...` (overwrites generated files).
3. Run `behave --dry-run`, then the requested regression against the application.
4. Report generated changes and test results; commit only when requested.

## Do not

- Hand-edit generated files without re-exporting (files are marked generated).
- Expect behave to call LLMs or teshi at runtime.
