# language: en

@web-ui
Feature: Browser profile selection
  As a tester using teshi web
  I want to see browser profiles and choose one myself
  So that teshi never acts against an ambiguous Chrome target

  Background:
    Given teshi web is running at http://127.0.0.1:20253/?e2e=1

  Scenario: Teshi web shows Browser Profiles after startup
    When the teshi web page finishes loading
    Then the Browser Profiles surface is shown

  Scenario: Multiple profiles are not selected automatically
    Given more than one browser profile is available
    When the teshi web page finishes loading
    Then no browser profile is selected automatically

  Scenario: User selects a browser profile explicitly
    Given more than one browser profile is available
    And no browser profile is selected
    When the user selects a browser profile
    Then that profile becomes the active browser target

  Scenario: User refreshes the browser profile list
    Given the Browser Profiles surface is shown
    When the user refreshes the browser profile list
    Then the profile list is updated from the current bridge

  Scenario: User can retry connecting Chrome from Browser Profiles
    Given the Chrome bridge is unavailable
    When the user requests a Chrome connection
    Then teshi web attempts to start the Chrome bridge
