# language: en
# Requirement: doc-26; partially bound; see test results for scenario status.
# Product decision approved.
@tui
Feature: Chat acp status row

  Scenario: shared activity row
    Given an ACP conversation performing a task
    When I view its status area
    Then the layout places the waiting animation and Agent controls on one row

  Scenario: narrow status priority
    Given a narrow terminal with an ACP approval request
    When I view the status area
    Then the layout keeps approval status discoverable and offers hidden settings through the settings control

  Scenario: Settings remain accessible when options overflow
    Given a narrow ACP chat with a long Agent name and several settings
    When I open the settings from the status row
    Then the settings remain accessible even when their individual chips do not fit

  Scenario: Keep error and interrupted-task recovery visible
    Given an ACP conversation with an error or interrupted task
    When I view its single status row
    Then the error or continuation control remains visible without an extra status row

  Scenario: Preserve the native backend status row
    Given a chat using the native backend
    When its task waits for approval
    Then the existing single status row still shows the approval request
