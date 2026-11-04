# language: en
@cli
Feature: Project requirement organization
  Background:
    Given an isolated requirement library with sample login and checkout documents
    And project A owns the sample requirements

  Scenario: Manual folders reference requirements without moving documents
    When the tester creates folder AI/Chat
    Then the command succeeds
    When the tester assigns the login requirement to AI/Chat
    Then the command succeeds
    And the folder hierarchy contains one login reference

  Scenario: Query collections preserve composed conditions
    When the tester creates a planned login query collection
    Then the command succeeds
    And the query retains both conditions

  Scenario: Invalid conditions fail explicitly
    When the tester creates a query with an invalid status
    Then the command fails

  Scenario: Organization is isolated by project identity
    When the tester creates folder AI/Chat
    Then the command succeeds
    When the tester reads project B organization
    Then the command succeeds
    And project B organization is empty
