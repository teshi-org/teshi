# language: en
@tui
Feature: Command line

  Background:
    Given Teshi is open

  Scenario Outline: Exit from the command line
    Given no buffer has unsaved edits
    And the command line prompt is open
    When I type "<command>" and press Enter
    Then the Teshi process exits successfully
    And the terminal screen is restored

    Examples:
      | command |
      | :q      |
      | :quit   |

  Scenario Outline: A save command writes the edited buffer before it stops
    Given a Feature buffer has unsaved edits
    And the command line prompt is open
    When I type "<command>" and press Enter
    Then the edited file is written to disk
    And Teshi <state>

    Examples:
      | command | state              |
      | :w      | keeps running      |
      | :wq     | exits successfully |
      | :x      | exits successfully |

  Scenario: An unsupported command does not run
    Given the command line prompt is open
    When I type ":zzz" and press Enter
    Then the footer reports an unknown command
    And the command line closes
    And Teshi keeps running

  Scenario: A colon stays literal inside text input
    Given the AI chat input has keyboard focus
    When I type ":q"
    Then the chat input shows ":q"
    And Teshi keeps running

  Scenario: Backspace removes the colon that opened the prompt
    Given the command line prompt is open
    When I press Backspace
    Then the command line closes
    And Teshi keeps running
