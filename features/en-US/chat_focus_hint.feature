# language: en
# Requirement: doc-29; source-pinned PTY case replay; see results.
@tui
Feature: Chat focus hint

  Scenario: Focus does not add border hints
    Given a chat input with token statistics
    When I focus and blur the input repeatedly
    Then only border highlighting changes and token statistics remain visible

  Scenario: Retain send and newline controls
    Given a focused chat input
    When I insert a newline then send the message
    Then the message preserves the newline and is sent once
