# language: en
# Requirement source: reqstore-18d3e4036ad71b3800002674/doc-6

@tui
Feature: Durable agent conversations
  Preserve native and ACP conversations across application restarts.

  Background:
    Given Teshi is open with an isolated project and conversation storage
    And I open AI chat

  Scenario: Save an unfinished reply without a lifecycle command (Native)
    Given the selected backend is "Native" with a controlled agent
    When I send the message "Inspect this project"
    And the agent streams reasoning "Inspecting files" and reply "Partial result" without finishing
    Then conversation storage contains the message "Inspect this project"
    And the saved conversation contains reasoning "Inspecting files" and reply "Partial result"
    When the agent extends its reply to "Partial result completed"
    Then the same saved conversation contains reply "Partial result completed"
    And conversation storage contains exactly one conversation

  Scenario: Save an unfinished reply without a lifecycle command (ACP)
    Given the selected backend is "ACP" with a controlled agent
    When I send the message "Inspect this project"
    And the agent streams reasoning "Inspecting files" and reply "Partial result" without finishing
    Then conversation storage contains the message "Inspect this project"
    And the saved conversation contains reasoning "Inspecting files" and reply "Partial result"
    When the agent extends its reply to "Partial result completed"
    Then the same saved conversation contains reply "Partial result completed"
    And conversation storage contains exactly one conversation

  Scenario: Recover a checkpoint after an unexpected termination (Native)
    Given the selected backend is "Native" with a controlled agent
    And a checkpoint contains the current unfinished conversation with question "Inspect this project", reasoning "Inspecting files", and reply "Partial result completed"
    When Teshi is terminated without a graceful exit
    And I reopen Teshi for the same project with the same conversation storage
    And I open AI chat
    And I resume the most recently updated conversation
    Then "Inspect this project" is visible in AI chat
    And "Inspecting files" is visible in AI chat
    And "Partial result completed" is visible in AI chat
    And reply "Partial result completed" has neutral styling without a green final-answer header or rail

  Scenario: Recover a checkpoint after an unexpected termination (ACP)
    Given the selected backend is "ACP" with a controlled agent
    And a checkpoint contains the current unfinished conversation with question "Inspect this project", reasoning "Inspecting files", and reply "Partial result completed"
    When Teshi is terminated without a graceful exit
    And I reopen Teshi for the same project with the same conversation storage
    And I open AI chat
    And I resume the most recently updated conversation
    Then "Inspect this project" is visible in AI chat
    And "Inspecting files" is visible in AI chat
    And "Partial result completed" is visible in AI chat
    And reply "Partial result completed" has neutral styling without a green final-answer header or rail

  Scenario: Restore ACP selection and execution history
    Given the selected backend is the controlled installed ACP agent "History agent"
    And the agent has completed a conversation about "Project Cedar"
    And the conversation includes reasoning "Inspecting Cedar" and completed tool "Read project"
    When I quit Teshi
    And I reopen Teshi for the same project with the same conversation storage
    And I open AI chat
    And I resume the most recently updated conversation
    Then the selected backend is still the ACP agent "History agent"
    And "Inspecting Cedar" is visible in AI chat
    And AI chat shows tool "Read project" with status "completed"

  Scenario: Supply saved context once to a new ACP process
    Given the selected backend is the controlled installed ACP agent "History agent"
    And a saved conversation contains a discussion about "Project Cedar" and completed tool "Read project"
    And I have resumed that conversation after reopening Teshi
    When I send the message "Which project were we discussing?"
    Then the new ACP process receives the saved discussion about "Project Cedar"
    And the new ACP process receives the current question
    And the new ACP process does not receive tool progress entries as conversation messages
    When the agent finishes the turn with the answer "Project Cedar"
    And I send the message "Continue the inspection"
    Then the same ACP process receives the current message without replaying the saved transcript again

  Scenario: Restore a legacy conversation without backend or project metadata
    Given conversation storage contains a legacy conversation with the message "Legacy question" and answer "Legacy answer"
    When I open the session browser
    Then the legacy conversation is not listed
    When I open the legacy session browser
    Then the legacy conversation is listed
    When I load the legacy conversation
    Then "Legacy question" is visible in AI chat
    And "Legacy answer" is visible in AI chat
    And the selected backend is "Native"
    And the legacy conversation remains unassigned

  Scenario: Keep newly saved conversations isolated by project
    Given a completed conversation "Cedar discussion" is saved for project "Cedar"
    And a completed conversation "Birch discussion" is saved for project "Birch"
    When I open Teshi for project "Cedar" with the same conversation storage
    And I open the session browser
    Then the conversation "Cedar discussion" is listed
    And the conversation "Birch discussion" is not listed

  Scenario: Reopen a cancelled conversation without promoting its partial reply (Native)
    Given the selected backend is "Native" with a controlled agent
    When I send the message "Cancel this inspection"
    And the agent streams reasoning "Inspecting before cancellation" and reply "Cancelled partial result" without finishing
    And the turn ends through "cancellation"
    Then the saved conversation contains reasoning "Inspecting before cancellation" and reply "Cancelled partial result"
    When I quit Teshi
    And I reopen Teshi for the same project with the same conversation storage
    And I resume the most recently updated conversation
    Then reply "Cancelled partial result" has neutral styling without a green final-answer header or rail
    And conversation storage contains exactly one conversation

  Scenario: Reopen a cancelled conversation without promoting its partial reply (ACP)
    Given the selected backend is "ACP" with a controlled agent
    When I send the message "Cancel this inspection"
    And the agent streams reasoning "Inspecting before cancellation" and reply "Cancelled partial result" without finishing
    And the turn ends through "cancellation"
    Then the saved conversation contains reasoning "Inspecting before cancellation" and reply "Cancelled partial result"
    When I quit Teshi
    And I reopen Teshi for the same project with the same conversation storage
    And I resume the most recently updated conversation
    Then reply "Cancelled partial result" has neutral styling without a green final-answer header or rail
    And conversation storage contains exactly one conversation

  Scenario: Report a checkpoint failure and recover without losing the conversation (Native)
    Given the selected backend is "Native" with a controlled agent
    And conversation storage is unavailable
    When I send the message "Recover this inspection"
    And the agent streams reasoning "Inspecting during a storage failure" and reply "Recoverable partial result" without finishing
    Then a conversation checkpoint failure is visible
    And "Recoverable partial result" is visible in AI chat
    When I restore writable conversation storage
    Then conversation storage contains the message "Recover this inspection"
    And the saved conversation contains reply "Recoverable partial result"
    When the agent extends its reply to "Recovery confirmed"
    Then the same saved conversation contains reply "Recovery confirmed"
    And conversation storage contains exactly one conversation

  Scenario: Report a checkpoint failure and recover without losing the conversation (ACP)
    Given the selected backend is "ACP" with a controlled agent
    And conversation storage is unavailable
    When I send the message "Recover this inspection"
    And the agent streams reasoning "Inspecting during a storage failure" and reply "Recoverable partial result" without finishing
    Then a conversation checkpoint failure is visible
    And "Recoverable partial result" is visible in AI chat
    When I restore writable conversation storage
    Then conversation storage contains the message "Recover this inspection"
    And the saved conversation contains reply "Recoverable partial result"
    When the agent extends its reply to "Recovery confirmed"
    Then the same saved conversation contains reply "Recovery confirmed"
    And conversation storage contains exactly one conversation

  Scenario: Keep native credentials out of saved conversations
    Given the selected backend is "Native" with a controlled agent
    And the controlled agent uses a synthetic credential
    When I send the message "Credential isolation probe"
    And the agent finishes the turn with the answer "Credential isolation complete"
    Then conversation storage contains the message "Credential isolation probe"
    And the saved conversation contains reply "Credential isolation complete"
    And the synthetic credential is absent from saved conversation data

  Scenario: Keep ACP credentials out of saved conversations
    Given the selected backend is "ACP" with a controlled agent
    And the controlled agent uses a synthetic credential
    When I send the message "Credential isolation probe"
    And the agent finishes the turn with the answer "Credential isolation complete"
    Then conversation storage contains the message "Credential isolation probe"
    And the saved conversation contains reply "Credential isolation complete"
    And the synthetic credential is absent from saved conversation data
