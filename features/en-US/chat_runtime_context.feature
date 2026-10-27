# language: en
# Requirement: doc-27; partially bound; see test results for scenario status.
@tui
Feature: Chat runtime context

  Scenario: Show task elapsed time across tool work
    Given a chat task that continues through tool work
    When I wait while the task runs
    Then the elapsed time increases and the final duration remains visible when the task ends

  Scenario: Unknown usage is not zero
    Given an ACP provider that has not reported input tokens
    When I view the chat input title
    Then context usage is shown as unknown

  Scenario: Show reported occupancy and threshold
    Given a provider reporting input usage above seventy percent of its context window
    When I view the chat input title
    Then the reported occupancy and window limit are shown with a warning indicator
