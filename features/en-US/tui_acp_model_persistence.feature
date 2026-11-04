# language: en

@tui
Feature: Remember the selected ACP model
  Each installed ACP agent remembers its last confirmed model selection.
  New sessions restore that model before sending their first prompt.

  Background:
    Given an isolated Teshi configuration with deterministic ACP agents "Fake Alpha" and "Fake Beta" installed
    And both agents advertise models "Alpha" and "Beta" with "Alpha" as the default
    And no agent backend environment override is set
    And Teshi is open in AI chat

  # test_point_ids: ACP-MODEL-TP1, ACP-MODEL-TP2
  Scenario Outline: Fresh sessions restore the confirmed model
    Given the current conversation uses "Fake Alpha"
    When I select the ACP model "Beta"
    Then the model change is confirmed
    When I <transition>
    Then the current conversation uses "Fake Alpha"
    And the confirmed ACP model is "Beta"
    When I send "verify restored model"
    Then a reply from "Fake Alpha" is visible
    And the ACP trace confirms model "Beta" before the first prompt in the new session

    Examples:
      | transition                                          |
      | create a new agent                                  |
      | execute the slash command "/new"                    |
      | restart Teshi with the same isolated configuration   |

  # test_point_ids: ACP-MODEL-TP3
  Scenario: Agent preferences and existing conversations remain independent
    Given the original conversation uses "Fake Alpha" with confirmed model "Beta"
    When I create a new agent
    And I select the ACP model "Alpha"
    Then the model change is confirmed
    When I return to the original conversation
    Then the confirmed ACP model is "Beta"
    When I select "Fake Beta" from the agent backend panel
    Then the confirmed ACP model is "Alpha"
    When I select the ACP model "Beta"
    Then the model change is confirmed
    When I create a new agent
    And I select "Fake Alpha" from the agent backend panel
    Then the confirmed ACP model is "Alpha"

  # test_point_ids: ACP-MODEL-TP4
  Scenario: A rejected selection does not replace the saved preference
    Given the current conversation uses "Fake Alpha"
    When I select the ACP model "Beta"
    Then the model change is confirmed
    Given "Fake Alpha" rejects changes to model "Alpha"
    When I select the ACP model "Alpha"
    Then the ACP settings report the rejected change
    And the confirmed ACP model is "Beta"
    When I restart Teshi with the same isolated configuration
    Then the confirmed ACP model is "Beta"

  # test_point_ids: ACP-MODEL-TP5
  Scenario: An unavailable saved model falls back to the agent default
    Given the current conversation uses "Fake Alpha"
    When I select the ACP model "Beta"
    Then the model change is confirmed
    Given "Fake Alpha" no longer advertises model "Beta" for new sessions
    When I restart Teshi with the same isolated configuration
    Then the ACP settings report that the saved model is unavailable
    And the confirmed ACP model is "Alpha"
    When I send "verify default model"
    Then a reply from "Fake Alpha" is visible
    And the ACP trace confirms model "Alpha" for that prompt

  # test_point_ids: ACP-MODEL-TP6
  Scenario: A restoration timeout blocks prompts until reconciliation
    Given "Fake Alpha" has a saved confirmed model preference "Beta"
    And "Fake Alpha" times out restoring model "Beta" in new sessions
    When I restart Teshi with the same isolated configuration
    Then the ACP settings report a model restoration timeout
    When I attempt to send "do not send with an uncertain model"
    Then the ACP trace contains no prompt for the uncertain session
    And the rejected message remains available in the input for retry
    Given "Fake Alpha" successfully accepts model changes again
    When I reconnect the current ACP conversation
    Then the confirmed ACP model is "Beta"
    When I send "verify reconciled model"
    Then a reply from "Fake Alpha" is visible
    And the reconciled ACP prompt excludes the rejected message
