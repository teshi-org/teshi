# language: en

@web-ui @embedded
Feature: Locator panel — confirm, reject, and unbind proposals
  Tests the bottom dock Locator panel where users confirm or reject
  locator proposals from the BDD agent. This is the human-in-the-loop
  step of the locator binding workflow.

  Background:
    Given teshi web is open with the current project
      And a Gherkin feature file is selected and a step is highlighted
      And a pending locator proposal exists for that step

  Scenario: Locator panel shows pending proposal with candidate details
    When the bottom dock Locator tab is active
    Then the Locator panel should display the pending proposal
      And the proposal should show the candidate strategy, selector, and action
      And the Confirm button should be enabled
      And the Reject button should be enabled

  Scenario: Confirm a pending locator proposal
    Given the Locator panel shows a pending proposal
    When I click the Confirm button
    Then the binding should be persisted to the step-bindings file
      And the step badge in the Gherkin panel should update to confirmed

  Scenario: Reject a pending locator proposal
    Given the Locator panel shows a pending proposal
    When I click the Reject button
    Then the pending proposal should be discarded
      And the Locator panel should show no pending proposal

  Scenario: Unbind an already-confirmed step
    Given the Gherkin panel shows a step with a confirmed binding
    When I click the Unbind button in the Locator panel
    Then the binding should be removed from the step-bindings file
      And the step badge should return to unbound status
