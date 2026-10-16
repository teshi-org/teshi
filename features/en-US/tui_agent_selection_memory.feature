# language: en

@tui
Feature: Remember the last explicitly selected agent
  New conversations reuse the last explicit backend selection across restarts.
  Existing conversations retain their own backend and history.

  Background:
    Given an isolated Teshi configuration with deterministic ACP agents "Fake Alpha" and "Fake Beta" installed
    And no agent backend environment override is set
    And Teshi is open in AI chat

  Scenario: First launch without a saved selection uses Native
    Then the current conversation uses Teshi Native

  # test_point_ids: TP1
  Scenario: A new agent uses the last explicitly selected ACP agent
    When I select "Fake Alpha" from the agent backend panel
    And I send "original conversation"
    Then a reply from "Fake Alpha" is visible
    When I create a new agent
    Then the current conversation uses "Fake Alpha"
    And the current conversation has no messages
    When I send "new agent conversation"
    Then a reply from "Fake Alpha" is visible
    And the fake ACP trace records a separate session for each conversation

  # test_point_ids: TP2
  Scenario: The new command retains the selected agent with a fresh session
    When I select "Fake Alpha" from the agent backend panel
    And I send "before new"
    Then a reply from "Fake Alpha" is visible
    When I execute the slash command "/new"
    Then the current conversation uses "Fake Alpha"
    And the current conversation has no messages
    When I send "after new"
    Then a reply from "Fake Alpha" is visible
    And the fake ACP trace records a separate session for each conversation
    And the new ACP session prompt excludes "before new"

  # test_point_ids: TP3
  Scenario: Restart remembers the most recent explicit ACP selection
    When I select "Fake Alpha" from the agent backend panel
    And I select "Fake Beta" from the agent backend panel
    And I restart Teshi with the same isolated configuration
    Then the current conversation uses "Fake Beta"
    And the current conversation has no messages
    When I send "after restart"
    Then a reply from "Fake Beta" is visible

  # test_point_ids: TP4
  Scenario: Explicitly selecting Native replaces the saved ACP preference
    When I select "Fake Alpha" from the agent backend panel
    And I select Teshi Native from the agent backend panel
    And I create a new agent
    Then the current conversation uses Teshi Native
    When I execute the slash command "/new"
    Then the current conversation uses Teshi Native
    When I restart Teshi with the same isolated configuration
    Then the current conversation uses Teshi Native

  # test_point_ids: TP5
  Scenario: Switching conversations preserves their backends and the explicit preference
    When I select "Fake Alpha" from the agent backend panel
    And I send "alpha history"
    Then a reply from "Fake Alpha" is visible
    When I create a new agent
    And I select "Fake Beta" from the agent backend panel
    And I switch to the original agent
    Then the current conversation uses "Fake Alpha"
    And "alpha history" is visible in the current conversation
    When I create a new agent
    Then the current conversation uses "Fake Beta"

  # test_point_ids: TP5
  Scenario: Restoring history does not replace the last explicit selection
    When I select "Fake Alpha" from the agent backend panel
    And I send "saved alpha history"
    Then a reply from "Fake Alpha" is visible
    When I execute the slash command "/new"
    And I select "Fake Beta" from the agent backend panel
    And I restore the saved conversation containing "saved alpha history"
    Then the current conversation uses "Fake Alpha"
    And "saved alpha history" is visible in the current conversation
    When I create a new agent
    Then the current conversation uses "Fake Beta"
    When I restart Teshi with the same isolated configuration
    Then the current conversation uses "Fake Beta"

  # test_point_ids: TP6
  Scenario: A removed preferred ACP agent produces a visible Native fallback
    When I select "Fake Alpha" from the agent backend panel
    And I quit Teshi
    And "Fake Alpha" is removed from the isolated installed agent store
    And I reopen Teshi with the same isolated configuration
    Then a warning identifies "Fake Alpha" as unavailable and reports the Native fallback
    And the current conversation uses Teshi Native

  # test_point_ids: TP6
  Scenario: A connection failure does not silently switch the preferred agent
    When I select "Fake Alpha" from the agent backend panel
    And I quit Teshi
    And the installed "Fake Alpha" is configured to fail at connection
    And I reopen Teshi with the same isolated configuration
    And I send "connection failure probe"
    Then an ACP connection error is visible
    And the current conversation uses "Fake Alpha"
    And no Native model request is recorded
