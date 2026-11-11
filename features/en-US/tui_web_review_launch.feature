# language: en

# Acceptance behavior; verified TUI step bindings and live replay remain pending.
@tui
Feature: Launch Web replay review from the screenshot viewer
  A user whose terminal cannot draw screenshot pixels opens the hosted Web UI
  from the viewer to inspect replay captures in a browser.

  Background:
    Given an isolated project with a recorded replay screenshot
    And the screenshot viewer is open

  Scenario: Open the hosted UI from the viewer
    Given a live daemon for the project
    When the user activates the Web control
    Then a hosted Web UI address is built from the daemon's actual port
    And the address contains a freshly minted hosted session token in its fragment
    And the address opens in the host browser
    And the same address is copied to the clipboard as plain text

  Scenario Outline: The Web control is reachable through both input modes
    Given a live daemon for the project
    When the user activates the Web control through <input>
    Then the hosted Web UI is launched

    Examples:
      | input            |
      | the top button   |
      | the w key        |

  Scenario: Keyboard and mouse share one action
    Given a live daemon for the project
    When the user activates the Web control through the top button
    Then the same launch behavior as the w key occurs

  Scenario: Start the project daemon on the first activation
    Given no live daemon for the project
    When the user activates the Web control
    Then a daemon is started for the project
    And a hosted Web UI address is built from that daemon's port
    And the address opens in the host browser

  Scenario: Reuse the daemon on a later activation
    Given the viewer already started the project daemon
    When the user activates the Web control again
    Then the running daemon is reused
    And no additional daemon is started
    And the new address uses the same port

  Scenario: Keep the address when the browser cannot be started
    Given a live daemon for the project
    And the host browser cannot be started
    When the user activates the Web control
    Then the status still shows the full hosted Web UI address
    And the address is copied to the clipboard

  Scenario Outline: Launch through the host platform's opener
    Given a live daemon for the project
    And the host platform is <platform>
    When the user activates the Web control
    Then the address is handed to <opener>

    Examples:
      | platform | opener           |
      | Windows  | cmd start        |
      | Termux   | termux-open-url  |
      | macOS    | open             |
      | Linux    | xdg-open         |

  Scenario: Reopening supersedes the previous hosted session
    Given a live daemon for the project
    When the user activates the Web control twice
    Then the second address carries a different session token
    And the earlier hosted page can no longer control the daemon
