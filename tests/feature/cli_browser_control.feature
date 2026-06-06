# language: en

@e2e
Feature: Browser automation via teshi CLI
  # Medium granularity: covers individual `teshi browser` subcommands —
  # sidecar lifecycle (serve-embedded, doctor, reconnect), navigation (navigate, snapshot),
  # element interrogation (highlight, clear-highlight), action execution (execute),
  # verification (verify), and batch replay (replay).
  # Stable contracts: src/cli/browser.rs handle_browser_command dispatch,
  #   src/cli/browser_endpoint.rs CdpEndpoint read/write,
  #   browser_service.py WebSocket cmd protocol (get_page_snapshot, navigate,
  #     highlight_selector, clear_highlight, execute_locator),
  #   .teshi/cdp-endpoint.json ws_url + mode fields,
  #   .teshi/step-bindings/{feature}.json replay source.

  Background:
    Given the teshi project root contains at least one .feature file with confirmed step bindings
      And the browser sidecar is running (via `teshi browser serve-embedded --project .`)

  Scenario: Doctor reports sidecar health
    When I run `teshi browser doctor`
    Then the output should indicate the sidecar is reachable and responsive

  Scenario: Navigate to a URL and snapshot the page
    Given the embedded browser is idle at a known starting page
    When I run `teshi browser navigate http://127.0.0.1:1421`
    Then the browser should navigate to that URL
    When I run `teshi browser snapshot`
    Then the output should contain page accessibility tree nodes and interactive elements

  Scenario: Highlight and clear a CSS selector
    When I run `teshi browser highlight h1`
    Then the browser page should show a highlight overlay on the h1 element
    When I run `teshi browser clear-highlight`
    Then the highlight overlay should disappear from the page

  Scenario: Execute a click action on a CSS selector
    Given the browser is on a page with a clickable button
    When I run `teshi browser execute --selector "button#submit" --action click`
    Then the button should have been clicked

  Scenario: Execute a fill action on an input field
    Given the browser is on a page with an input field
    When I run `teshi browser execute --selector "input[name=email]" --action fill --value-arg "test@example.com"`
    Then the input field should contain the text "test@example.com"

  Scenario: Verify a locator and append to the verification log
    Given a known CSS selector for a visible element
    When I run `teshi browser verify --step-line 42 --selector "h1.welcome" --action assert_visible`
    Then the output should confirm the selector was highlighted and executed successfully
      And a verification record should be appended to .teshi/logs/locator-verify.jsonl

  Scenario: Replay confirmed step bindings through the browser
    Given a feature file with multiple confirmed step bindings
    When I run `teshi browser replay --feature tests/feature/my_scenario.feature --non-interactive`
    Then each binding should be executed in order, with output indicating pass/fail per step

  Scenario: Reconnect restarts the embedded sidecar
    Given a running but unhealthy sidecar
    When I run `teshi browser reconnect`
    Then a new sidecar process should be started
      And .teshi/cdp-endpoint.json should be refreshed with the new ws_url

  Scenario: Serve-embedded starts a sidecar and writes the endpoint file
    When I run `teshi browser serve-embedded --project . --navigate http://127.0.0.1:1421`
    Then .teshi/cdp-endpoint.json should exist with a valid ws_url and mode "embedded"
      And the sidecar should be listening on the WebSocket port
