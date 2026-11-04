# language: en

@tui
Feature: Long ACP conversation
  Keep current ACP responses visible throughout an extended wrapped conversation.

  Background:
    Given Teshi is open with a deterministic ACP agent for a long conversation

  Scenario: Complete thirty wrapped conversation turns
    Given I open AI chat for the long conversation
    When I complete wrapped conversation turns 1 through 10
    Then wrapped ACP reply 10 is visible
    When I complete wrapped conversation turns 11 through 20
    Then wrapped ACP reply 20 is visible
    When I complete wrapped conversation turns 21 through 30
    Then wrapped ACP reply 30 is visible
    When I leave the long conversation input
    And I quit the long conversation
    Then the long conversation exits successfully
