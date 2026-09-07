# language: en

@cli
Feature: Requirement authoring
  As a tester or agent using the requirement library
  I want to assign iterations and replace document bodies
  So that I can update the store without opening the TUI

  Sample documents:
  - doc-12 auth/login.md titled Login in Sprint 12
  - doc-37 mobile/login.md titled Login, unassigned
  - doc-9 shop/checkout.md titled Checkout, unassigned

  Background:
    Given an isolated requirement library with sample login and checkout documents

  Scenario: Batch assignment moves both documents into an iteration
    When the tester assigns documents "doc-12" and "doc-9" to iteration "Sprint 13"
    Then the command succeeds
    And documents "doc-12" and "doc-9" belong to iteration "Sprint 13"

  Scenario: Unknown document in a batch leaves existing assignments unchanged
    When the tester assigns documents "doc-12" and "missing" to iteration "Sprint 13"
    Then the command fails
    And document "doc-12" belongs to iteration "Sprint 12"

  Scenario: Clearing iteration returns documents to unassigned
    Given documents "doc-12" and "doc-9" belong to iteration "Sprint 13"
    When the tester clears the iteration on documents "doc-12" and "doc-9"
    Then the command succeeds
    And documents "doc-12" and "doc-9" are unassigned

  Scenario: File replacement updates the login document body
    When the tester replaces the body of document "doc-12" with:
      """
      # Login

      Updated authentication flow.
      """
    Then the command succeeds
    And the body of document "doc-12" is:
      """
      # Login

      Updated authentication flow.
      """

  Scenario: Unchanged replacement does not rewrite revision
    When the tester resubmits the current body of document "doc-12"
    Then the command succeeds
    And the revision of document "doc-12" is unchanged

  Scenario: Non-interactive edit without a body is rejected
    When the tester edits document "doc-12" as JSON without providing a body
    Then the command fails
    And the JSON error code is "missing_edit_input"

  Scenario: Blank iteration name is rejected
    When the tester assigns document "doc-12" to iteration "   " as JSON
    Then the command fails
    And the JSON error code is "invalid_iteration_name"
