# language: en

@winapp
Feature: WinApp negative UIA assertions
  Confirm that a native selector can be asserted absent.

  Scenario: Missing controls can be asserted absent
    Given the teshi desktop window is visible
    Then the missing native marker is absent
