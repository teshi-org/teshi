# language: en

@tui
Feature: ACP session settings are reachable by mouse and keyboard
  An ACP agent advertises its settings when the session starts, and the bottom
  control row exposes them without leaving the AI chat tab.

  Background:
    Given Teshi is open with a deterministic ACP agent that advertises settings

  Scenario: Switching the model refreshes the choices that depend on it
    When I open AI chat for the settings conversation
    And I open the agent settings list
    Then the advertised model and overflow controls are visible
    When I click the model control in the bottom row
    And I choose the second model option
    Then the model change is confirmed
    And the dependent thinking choices are refreshed
    When I narrow the terminal to 64 columns
    Then the settings row still offers an overflow control
    When I turn on the fast option from the settings list
    Then the fast option stays on
    When I leave the chat input
    And I quit Teshi
    Then Teshi exits successfully