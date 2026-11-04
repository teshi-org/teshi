# language: en

# Acceptance behavior; verified UI bindings and live replay remain pending.
# Intended verification route: GPUI WASM browser E2E, with native GPUI parity review.
@web-ui
Feature: Replay screenshot review
  Users review screenshots associated with individual Feature replay steps
  in the GPUI Run view and retain evidence across separate runs.

  Background:
    Given teshi web is running at http://127.0.0.1:20253/?e2e=1&replay_review=1
    And teshi web has opened an isolated project containing a replayable Feature
    And the user has opened the Run view

  Scenario: View a screenshot for a completed step
    Given the selected browser replay supports screenshots
    When the user starts the selected scenario
    And a step completes successfully
    And the user selects that step
    Then the screenshot panel shows the image captured after that step
    And the panel identifies the Feature, scenario, step text, and capture time

  Scenario: Inspect an available screenshot while replay continues
    Given a replay has captured a screenshot for its first step
    And a later step is still running
    When the user selects the first step
    Then the screenshot panel shows the first step's saved image
    And the run remains in progress

  Scenario: Retain the screenshot of a failed step
    Given the selected native application replay supports screenshots
    And a step will fail its assertion while the target remains available for capture
    When the user starts the selected scenario
    And the replay stops at the failed step
    And the user selects the failed step
    Then the step is marked as failed
    And the screenshot panel shows the image captured at the failed step
    And screenshots from earlier completed steps remain available

  Scenario: Enlarge a step screenshot
    Given the screenshot panel shows a saved step image
    When the user opens the image at full size
    Then an enlarged image preview is displayed
    When the user closes the enlarged preview
    Then the same step remains selected
    And its screenshot remains visible in the panel

  Scenario: Keep screenshots separate between runs of the same Feature
    Given a completed replay has saved step screenshots
    When the user runs the same scenario again
    And the new replay captures step screenshots
    And the user selects the earlier run
    Then the earlier run shows its original screenshots
    When the user selects the new run
    Then the new run shows only its own screenshots

  Scenario: Restore screenshot associations after reopening the project
    Given a finished replay has saved screenshots for completed and failed steps
    When the user closes the project
    And the user reopens the same project
    And the user selects that run from its history
    And the user selects a step with a saved screenshot
    Then the screenshot panel shows that step's original image
    And the run retains its original outcome

  Scenario: Report a capture failure without changing the step result
    Given a replay step succeeds but its screenshot capture fails
    When the user selects that step
    Then the step is marked as passed
    And the screenshot panel explains that capture failed
    And the panel does not show an image from another step or run

  Scenario: Explain when the replay backend does not support screenshots
    Given the selected replay backend does not support screenshots
    When the user starts the selected scenario
    And the user selects a completed step
    Then the screenshot panel explains that this backend does not support screenshots
    And the step result reflects its execution outcome

  Scenario: Explain when a saved image is missing or unreadable
    Given a finished run contains a screenshot association for a step
    And the associated image is missing or unreadable
    When the user selects that step
    Then the screenshot panel explains that the saved image cannot be loaded
    And the step result remains visible
    And the user can select another step

  Scenario: Keep screenshot evidence scoped to the selected project
    Given two projects contain a Feature with the same name and step line numbers
    And both projects have saved replay screenshots
    When the user switches to the second project
    And the user selects a saved replay step
    Then the screenshot panel shows the image associated with that step in the second project
    And the panel does not retain an image from the first project
