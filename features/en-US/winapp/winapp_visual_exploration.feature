# language: en
# Requirement: doc-21; authored only; not bound, executed or accepted.
@winapp
Feature: Winapp visual exploration

  Scenario: Operate without semantic controls
    Given an attached isolated window with no useful UIA controls and foreground input permission
    When I observe the client and perform a coordinate click followed by text input
    Then the plan records target identity, physical coordinates, screenshots and per-step durations without claiming business success

  Scenario: Reject stale coordinates
    Given a client observation at 125 percent scaling
    When the window moves and I submit input using the old observation
    Then input is rejected until I observe the window again

  Scenario: Refuse unpermitted physical input
    Given an attached window in background mode
    When I request a coordinate click
    Then the request fails before any physical input occurs

  Scenario: Stop an uncertain sequence
    Given a visual plan with several actions
    When one action has an uncertain result
    Then later actions are not dispatched and the journal retains completed steps without automatic retry

  Scenario: Reject uncalibrated external success
    Given an external checker with unverified thresholds
    When the checker reports a passed result
    Then Teshi rejects the result as unverified and retains its evidence

  Scenario: Bound external checker execution
    Given an external checker that does not finish
    When its execution deadline expires
    Then the check fails with a timeout and does not automatically retry

  Scenario: Capture failure preserves the action result
    Given a foreground-authorized visual plan with two mutations
    When the first mutation completes but its following screenshot fails
    Then its provider result remains recorded and the second mutation is not dispatched

  Scenario: Stable pixels do not assert business success
    Given a visual plan observing an unchanged window
    When the view remains stable within the requested deadline
    Then stability is reported while the business result remains unverified

  Scenario: Scroll and drag within an observed client
    Given an authorized controlled window with scrollable and draggable content
    When I scroll and drag inside its observed client area
    Then separate step results and screenshots are retained for later business verification
