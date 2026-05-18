# language: en

@e2e
Feature: Run BDD scenarios and view test results
  # Coarse granularity: covers the end-to-end chain of configuring the Runner → triggering a run →
  #   receiving NDJSON events → updating UI status.
  # Stable contracts: `teshi run` CLI subcommand (runner::run_cli), `runner::spawn_runner` NDJSON
  #   protocol, `RunEvent::CasePassed/CaseFailed/EndRun` event stream, `apply_run_event` updating
  #   explore_case_status, RunStatus color mapping in the UI (status_color)

  Background:
    Given the project directory contains a valid teshi.toml configuration file with a runner command
      And the project directory contains a feature file with one passing scenario and one failing scenario

  Scenario: Run a single scenario in the Explore tab and observe status transitions
    Given the TUI is running on the Explore tab
      And I have selected a Scenario that has not yet been run
      And that Scenario's status is displayed as "pending" (gray)
    When I press `r`
    Then that Scenario's status should change to "running" (yellow)
      And its steps' statuses should also change to "running" (yellow)
    When the Runner process returns results
    Then for a passing scenario, the Scenario and step statuses should change to "passed" (green)
      And for a failing scenario, the Scenario and step statuses should change to "failed" (red)
      And pressing `Enter` on a failed step should expand to show error details
          (including the error message and stack trace)

  Scenario: Run tests from the CLI subcommand and inspect NDJSON output
    When I run `teshi run --feature login.feature --scenario "Successful login"` in the terminal
    Then standard output should contain NDJSON-formatted test event lines
      And the output should contain a `"type":"start_run"` event
      And the output should contain a `"type":"case_passed"` or `"type":"case_failed"` event
      And the output should contain a `"type":"end_run"` event with passed/failed/skipped counts
