# language: en

@winapp
Feature: WinApp negative UIA assertions
  Confirm that a native selector can be asserted absent.

  Scenario: Missing controls can be asserted absent
    Given the teshi desktop window is visible
    Then the missing native marker is absent

# Acceptance note (2026-10-16): replay is blocked because no Teshi Desktop
# window is attached. Start Desktop and attach through a WinApp executor
# in the same interactive Windows session, then rerun the real UI assertions.
