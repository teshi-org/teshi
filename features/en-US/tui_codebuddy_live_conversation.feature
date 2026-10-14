# language: en

@tui
Feature: Live CodeBuddy ACP conversation
  Keep real CodeBuddy ACP responses visible across ten wrapped conversation turns.

  Background:
    Given Teshi is open with the authenticated live CodeBuddy ACP agent

  Scenario: Complete ten real CodeBuddy turns
    Given I open AI chat for the live CodeBuddy conversation
    When I send live CodeBuddy conversation turn 1
    Then live CodeBuddy reply 1 is visible
    When I send live CodeBuddy conversation turn 2
    Then live CodeBuddy reply 2 is visible
    When I send live CodeBuddy conversation turn 3
    Then live CodeBuddy reply 3 is visible
    When I send live CodeBuddy conversation turn 4
    Then live CodeBuddy reply 4 is visible
    When I send live CodeBuddy conversation turn 5
    Then live CodeBuddy reply 5 is visible
    When I send live CodeBuddy conversation turn 6
    Then live CodeBuddy reply 6 is visible
    When I send live CodeBuddy conversation turn 7
    Then live CodeBuddy reply 7 is visible
    When I send live CodeBuddy conversation turn 8
    Then live CodeBuddy reply 8 is visible
    When I send live CodeBuddy conversation turn 9
    Then live CodeBuddy reply 9 is visible
    When I send live CodeBuddy conversation turn 10
    Then live CodeBuddy reply 10 is visible
    When I leave the live CodeBuddy input
    And I quit the live CodeBuddy conversation
    Then the live CodeBuddy conversation exits successfully
