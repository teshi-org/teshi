# language: en

@web-ui @embedded
Feature: Browser panel smoke — embedded browser controls
  Self-bootstrap smoke: teshi desktop/web hosts the IDE; the Browser panel's
  embedded Playwright preview is the AUT container. Tests the Start Embedded
  button, address bar navigation, and basic panel chrome interaction.

  Background:
    Given teshi web is open with the current project
      And the Browser panel is visible

  Scenario: Start Embedded browser and see the status indicator
    When I click the Start Embedded button
    Then the Embedded browser should start
      And the status indicator should show "connected"

  Scenario: Navigate to a URL using the address bar
    Given the Embedded browser is running
    When I type "http://127.0.0.1:1421" in the address bar
    And I click the Go button
    Then the browser should navigate to "http://127.0.0.1:1421"
    And the address bar should display the current URL

  Scenario: Zoom in and zoom out controls
    Given the Embedded browser is running
    When I click the Zoom In button
    Then the zoom level display should increase
    When I click the Zoom Out button
    Then the zoom level display should decrease
    When I click the Fit button
    Then the zoom level should reset to 100%

  Scenario: Fullscreen toggle expands the Browser panel
    Given the Embedded browser is running
    When I click the Fullscreen button
    Then the Browser panel should expand to fill the workspace
      And the Gherkin and Files panels should be hidden
    When I press the Escape key
    Then the workspace should return to the three-panel layout

  Scenario: Disconnect stops the Embedded browser
    Given the Embedded browser is running
    When I click the Disconnect button
    Then the Embedded browser should stop
      And the Start Embedded button should reappear
