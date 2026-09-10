# language: en

@cli
@validation-e2e
Feature: Gherkin validation self-bootstrap
  As a Teshi user
  I want `teshi check` to validate Feature source before execution
  So that syntax errors are actionable and valid files remain usable

  Background:
    Given an isolated Feature validation project

  Scenario: Missing Chinese step separator is diagnosed
    Given the project contains a malformed zh-CN Feature
    When the tester runs teshi check for the malformed Feature as JSON
    Then the command fails with exit code 1
    And the JSON report contains diagnostic code "missing_step_separator" at line 4 column 6
    And the diagnostic suggestion is "当 用户登录"

  Scenario: Valid source and attachments are accepted
    Given the project contains a valid Feature with prose and step attachments
    When the tester runs teshi check for the valid Feature as JSON
    Then the command succeeds
    And the JSON report has zero errors
    And the JSON report contains no unrecognized executable line

  Scenario: Valid Chinese source is accepted
    Given the project contains a valid zh-CN Feature
    When the tester runs teshi check for the valid zh-CN Feature as JSON
    Then the command succeeds
    And the JSON report has zero errors

  Scenario: Unrecognized executable text is rejected
    Given the project contains a Feature with unrecognized executable text
    When the tester runs teshi check for that Feature as JSON
    Then the command fails with exit code 1
    And the JSON report contains diagnostic code "unrecognized_executable_line" at line 4 column 5

  Scenario: Explicit scope is limited to the selected Feature
    Given the project contains one valid and one invalid Feature
    When the tester runs teshi check for the invalid Feature as JSON
    Then the command fails with exit code 1
    And the JSON scope is ["features/invalid.feature"]
    And the JSON report contains diagnostic code "missing_step_separator" at line 3 column 10

  Scenario: All scope reports ordered diagnostics
    Given the project contains one valid and one invalid Feature
    When the tester runs teshi check for all Features as JSON
    Then the command fails with exit code 1
    And the JSON scope is ["features/invalid.feature", "features/valid.feature"]
    And the all-scope JSON report contains ordered diagnostics

  Scenario: Warnings do not fail validation
    Given the project contains a warning-only Feature
    When the tester runs teshi check for the warning-only Feature as JSON
    Then the command succeeds
    And the warning code scenario_starts_without_given is visible

  Scenario: Legal repeated Given steps remain non-blocking
    Given the project contains a repeated-Given Feature
    When the tester runs teshi check for the repeated-Given Feature as JSON
    Then the command succeeds
    And the JSON report contains warning codes "missing_when" and "missing_then"

  Scenario: Directory run ignores invalid sibling Features
    Given the project contains a valid selected directory and an invalid sibling Feature
    When the tester starts the BDD run for the selected directory
    Then the command succeeds
    And the selected directory run succeeds without sibling diagnostics

  Scenario: Conflicting check scopes are rejected
    Given the project contains an invalid Feature
    When the tester runs teshi check with conflicting scope options
    Then the command fails with exit code 2
    And the output explains that scope options conflict

  Scenario: Unbound refuses an invalid Feature
    Given the project contains an invalid Feature
    When the tester lists unbound steps for the invalid Feature
    Then the command fails with exit code 1
    And the output contains missing_step_separator validation evidence
    And the binding command did not return a successful empty list

  Scenario: Next unbound preserves the active step on validation failure
    Given the project has an existing active step and an invalid Feature
    When the tester advances to the next unbound step for the invalid Feature
    Then the command fails with exit code 1
    And the output contains missing_step_separator validation evidence
    And the existing active step remains unchanged

  Scenario: BDD run does not start for an invalid Feature
    Given the project contains an invalid Feature
    When the tester starts BDD run for the invalid Feature
    Then the command fails with exit code 1
    And the output contains missing_step_separator validation evidence
    And the nested runner was not started

  Scenario: Replay entry points fail before target actions
    Given the project contains an invalid Feature
    When the tester starts browser and WinApp replay for the invalid Feature
    Then both replay commands failed before target action

  Scenario: Repeated validation reports are deterministic
    Given the project contains a malformed zh-CN Feature
    When the tester runs the malformed check twice as JSON
    Then the command fails with exit code 1
    And the two JSON reports are identical

  Scenario: Daemon warning report is structured and visible
    Given the project contains a warning-only Feature and a live daemon
    When the tester lists unbound steps for the warning-only Feature through the daemon
    Then the command succeeds
    And the daemon warning report is structured and visible

  Scenario: Daemon validation errors keep a structured report
    Given the project contains an invalid Feature and a live daemon
    When the tester lists unbound steps for the invalid Feature through the daemon
    Then the command fails with exit code 1
    And the daemon error contains a structured validation report
