# language: en

@tui
Feature: ACP permission prompt rejection stops the turn
  Rejecting an ACP agent's permission request ends the turn without the
  follow-up reply the agent would have produced.

  Background:
    Given Teshi is open with a deterministic ACP agent in a narrow terminal

  Scenario: Rejecting the permission ends the turn
    Given I open AI chat for the permission conversation
    When I send permission conversation turn 1
    Then the ACP permission prompt is visible
    When I reject the permission request
    Then the permission follow-up is not visible
    When I leave permission chat input
    And I quit permission chat
    Then permission chat exits successfully
