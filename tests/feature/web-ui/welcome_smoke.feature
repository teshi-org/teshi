# language: en

@web-ui @embedded
Feature: teshi web welcome screen smoke
  Self-bootstrap smoke: teshi desktop hosts the IDE; teshi web on loopback :1421 is the AUT.

  Background:
    Given teshi web is running at http://127.0.0.1:1421

  Scenario: Welcome screen shows Open Project
    Then the Open Project button should be visible
