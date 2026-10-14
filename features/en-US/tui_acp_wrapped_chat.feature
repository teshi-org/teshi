# language: en

@tui
Feature: Wrapped ACP chat replies
  Keep the newest ACP response visible after several long conversation turns.

  Background:
    Given Teshi is open with a deterministic ACP agent in a narrow terminal

  Scenario: Keep the sixth wrapped reply visible
    Given I open AI chat for the wrapped conversation
    When I send wrapped conversation turn 1
    Then wrapped ACP reply 1 is visible
    When I send wrapped conversation turn 2
    Then wrapped ACP reply 2 is visible
    When I send wrapped conversation turn 3
    Then wrapped ACP reply 3 is visible
    When I send wrapped conversation turn 4
    Then wrapped ACP reply 4 is visible
    When I send wrapped conversation turn 5
    Then wrapped ACP reply 5 is visible
    When I send wrapped conversation turn 6
    Then wrapped ACP reply 6 is visible
    When I leave wrapped chat input
    And I quit wrapped chat
    Then wrapped chat exits successfully
