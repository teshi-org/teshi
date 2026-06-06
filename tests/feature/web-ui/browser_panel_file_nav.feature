# language: en

@web-ui @embedded
Feature: Browser panel interaction with file tree and terminal
  End-to-end flow: use the Browser panel to navigate to a project page,
  switch to the Files tab to verify file content, then use the Terminal
  tab and type a command. Demonstrates the CLI's ability to control
  desktop/web buttons and keyboard input.

  Background:
    Given teshi web is open with the current project
      And the Embedded browser is running and navigated to "http://127.0.0.1:1421"

  Scenario: Switch to Files tab and select a feature file
    When I click the Files tab
    Then the file tree should be visible
    When I click a feature file node in the file tree
    Then the Gherkin panel should display that feature's scenarios and steps

  Scenario: Switch to Terminal tab and type a command
    Given the Embedded browser is navigated to the SUT page
    When I click the Terminal tab
    Then the terminal host should be visible
    When I type "echo Hello from teshi E2E" in the terminal
    Then the terminal output should contain "Hello from teshi E2E"

  Scenario: Restart the terminal shell
    Given the Terminal tab is active
    When I click the Restart Shell button
    Then the terminal should restart
      And the terminal should show a fresh shell prompt

  Scenario: Verify a file appears after a new project file is created
    Given the Terminal tab is active and the shell is ready
    When I type "New-Item .e2e-browser-test -ItemType File" in the terminal
    Then the terminal should show the creation confirmation
    When I click the Files tab
    Then the file tree should contain a node for ".e2e-browser-test"
