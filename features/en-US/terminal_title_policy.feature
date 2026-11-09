# language: en
# Requirement: doc-73; authored only; not bound, executed or accepted.
# Expectations describe the implemented policy; bind and replay before claiming acceptance.
@tui
Feature: Terminal title policy

  Scenario: Show progress and project while running
    Given one agent conversation is running
    When I view the terminal title
    Then it contains a running marker and the project without exposing conversation text

  Scenario: Aggregate running state across conversations
    Given three agent conversations with one running
    When I view the terminal title
    Then it shows one of three conversations running

  Scenario: Switch to a waiting marker for approval
    Given one agent conversation is waiting for user approval
    When I view the terminal title
    Then it shows a waiting marker instead of a running marker

  Scenario: Keep the result marker after a turn ends
    Given an agent conversation turn has ended
    When I view the terminal title
    Then it shows a success or failure marker

  Scenario: Restore the title on exit
    Given Teshi has updated the terminal title
    When I quit Teshi
    Then the terminal title is reset to empty

  Scenario: Leave the title untouched when disabled
    Given the user set TESHI_NO_TITLE
    When I start Teshi and run an agent conversation
    Then Teshi does not modify the terminal title
