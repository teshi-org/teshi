# language: en

@tui
Feature: ACP permission prompt allows the turn to continue
  A tool-using ACP agent pauses for a permission decision instead of
  aborting the turn after its first streamed message.

  Background:
    Given Teshi is open with a deterministic ACP agent in a narrow terminal

  Scenario: Allowing the second option continues the conversation
    Given I open AI chat for the permission conversation
    When I send permission conversation turn 1
    Then the first permission reply is visible
    And the ACP permission prompt is visible
    When I move the permission selection down
    And I confirm the permission selection
    Then the allowed permission reply is visible
    When I leave permission chat input
    And I quit permission chat
    Then permission chat exits successfully
