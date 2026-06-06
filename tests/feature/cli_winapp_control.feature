# language: en

@e2e
Feature: WinUI3 native window automation via teshi CLI
  # Medium granularity: covers individual `teshi winapp` subcommands —
  # target discovery (list-windows), attachment (attach, launch),
  # element interrogation (snapshot, highlight, clear-highlight),
  # action execution (execute), and batch replay (replay).
  # Stable contracts: src/cli/winapp.rs handle_winapp_command dispatch,
  #   winapp_service.py WebSocket cmd protocol (list_windows, attach_window,
  #     launch_app, get_ui_snapshot, highlight_selector, clear_highlight,
  #     execute_locator, get_target),
  #   .teshi/cdp-endpoint.json ws_url + mode "winapp" fields,
  #   UIA selector format "uia:automation_id=..." / "uia:control_type=...;name=..." / "uia:path=N/M/..."

  Background:
    Given the teshi project root contains at least one .feature file with confirmed UIA step bindings
      And the winapp sidecar is reachable

  Scenario: List visible top-level windows
    When I run `teshi winapp list-windows`
    Then the output should list at least one visible top-level window
      And each entry should include the window handle (hwnd) and title

  Scenario: Attach to a window by title fragment
    When I run `teshi winapp attach --title "Calculator"`
    Then the sidecar should be attached to the target window
      And `teshi winapp snapshot` should return its UIA tree

  Scenario: Attach to a window by process name
    When I run `teshi winapp attach --process-name "notepad"`
    Then the sidecar should attach to the first matching window
      And subsequent snapshot and execute commands should target that window

  Scenario: Launch an application and attach automatically
    When I run `teshi winapp launch "C:\Windows\System32\notepad.exe"`
    Then the application should start and the sidecar should attach to its main window
      And `teshi winapp snapshot` should return the UIA tree for the new window

  Scenario: Highlight and clear a UIA selector
    Given the sidecar is attached to a window with known UIA elements
    When I run `teshi winapp highlight "uia:automation_id=TitleBar"`
    Then a highlight overlay should appear on the identified element for approximately 3.5 seconds
    When I run `teshi winapp clear-highlight`
    Then the highlight overlay should be removed

  Scenario: Execute a click on a UIA element
    Given the sidecar is attached to a window with a clickable button
    When I run `teshi winapp execute --selector "uia:automation_id=SearchBox" --action click`
    Then the UIA element should receive a click and respond accordingly

  Scenario: Execute a fill on a text input element
    Given the sidecar is attached to a window with an editable text field
    When I run `teshi winapp execute --selector "uia:control_type=EditControl" --action fill --value-arg "hello world"`
    Then the text field should contain the text "hello world"

  Scenario: Replay confirmed UIA step bindings on the attached application
    Given the sidecar is attached to a target application window
      And a feature file with confirmed UIA step bindings exists
    When I run `teshi winapp replay --feature tests/feature/my_winapp_scenario.feature --non-interactive`
    Then each UIA binding should be executed in order
      And the output should report pass or failure for each step
