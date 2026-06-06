# language: en

@e2e
Feature: Gherkin step binding lifecycle via teshi CLI
  # Medium granularity: covers `teshi steps` subcommand workflow —
  # selecting a step, proposing locator candidates, confirming/rejecting them,
  # and querying binding status.
  # Stable contracts: src/cli/steps.rs handle_steps_command dispatch,
  #   crates/teshi-runtime/src/locator.rs ActiveStep / PendingLocator /
  #     LocatorCandidate / StepBinding / StepBindingsFile structs,
  #   .teshi/active-step.json, .teshi/pending-locator.json,
  #   .teshi/step-bindings/{sanitized_name}.json file paths.

  Background:
    Given the teshi project root contains a .feature file with at least two Gherkin steps
      And none of those steps have confirmed bindings yet

  Scenario: Select a Gherkin step and verify active-step.json
    When I run `teshi steps select --feature tests/feature/login.feature --line 15`
    Then .teshi/active-step.json should be written
      And its content should contain the feature path "tests/feature/login.feature"
      And its content should contain the step line number 15
      And its content should contain the corresponding step keyword and text

  Scenario: List unbound steps for a feature
    When I run `teshi steps unbound --feature tests/feature/login.feature`
    Then the output should list all steps without confirmed bindings
      And each entry should show the step line number, keyword, and step text

  Scenario: Select the next unbound step automatically
    Given the first step (line 10) already has a confirmed binding
    When I run `teshi steps next-unbound --feature tests/feature/login.feature`
    Then .teshi/active-step.json should reference the next unbound step (line 15)

  Scenario: Propose a locator candidate for the active step
    Given .teshi/active-step.json references a valid step
    When I run `teshi steps propose --strategy css --value "button.login" --action click --rank 1 --confidence 0.95`
    Then .teshi/pending-locator.json should be written
      And it should contain the candidate strategy, value, action, rank, and confidence

  Scenario: Confirm a pending locator and persist the binding
    Given .teshi/pending-locator.json contains a valid proposal
    When I run `teshi steps confirm --rank 1`
    Then the proposal should be persisted to .teshi/step-bindings/{feature}.json
      And .teshi/pending-locator.json should be deleted
      And the binding should include the strategy, selector, action, and a timestamp

  Scenario: Reject a pending locator
    Given .teshi/pending-locator.json contains a valid proposal
    When I run `teshi steps reject`
    Then .teshi/pending-locator.json should be deleted
      And .teshi/step-bindings/{feature}.json should not contain a binding for that step

  Scenario: Override the selector value during confirm
    Given .teshi/pending-locator.json contains a proposal with value "button.login"
    When I run `teshi steps confirm --rank 1 --selector "button#login-btn"`
    Then the persisted binding should have the selector "button#login-btn" (overridden)

  Scenario: Wait for a pending proposal to be confirmed
    Given .teshi/pending-locator.json contains a valid proposal
    When I run `teshi steps wait --until confirmed --timeout 30`
      And someone confirms the proposal externally
    Then the wait command should exit successfully
      And the binding should be present in .teshi/step-bindings/{feature}.json

  Scenario: Wait for a pending proposal to be rejected
    Given .teshi/pending-locator.json contains a valid proposal
    When I run `teshi steps wait --until rejected --timeout 30`
      And someone rejects the proposal externally
    Then the wait command should exit successfully
      And .teshi/pending-locator.json should not exist

  Scenario: Resolve confirmed bindings for a feature
    Given .teshi/step-bindings/login.json contains bindings for steps 10, 15, and 20
    When I run `teshi steps resolve --feature tests/feature/login.feature`
    Then the output should contain bindings for lines 10, 15, and 20
      And each binding should show the step text, selector, and action

  Scenario: Resolve bindings up to a specific line
    Given bindings exist for steps 10, 15, and 20
    When I run `teshi steps resolve --feature tests/feature/login.feature --until-line 15`
    Then the output should contain bindings for lines 10 and 15
      But should not contain the binding for line 20

  Scenario: List bindings with status for a feature
    Given some steps have confirmed bindings and others do not
    When I run `teshi steps list --feature tests/feature/login.feature`
    Then the output should show each step with its binding status
      And confirmed steps should show "confirmed" status
      And unbound steps should show "unbound" status

  Scenario: Unbind a specific step
    Given line 15 has a confirmed binding in .teshi/step-bindings/login.json
    When I run `teshi steps unbind --feature tests/feature/login.feature --line 15`
    Then the binding for line 15 should be removed from .teshi/step-bindings/login.json
      And `teshi steps list --feature tests/feature/login.feature` should show step 15 as unbound
