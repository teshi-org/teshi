# language: en
# Requirement: doc-20; authored only; not bound, executed or accepted.
@winapp
Feature: Winapp owned instances

  Scenario: Observe redirected second launch
    Given an owned primary instance using an isolated profile
    When I launch another owned instance with the same profile and wait for its exit
    Then the second instance exits with zero, has no windows and the primary remains alive

  Scenario: Stop only the owned instance
    Given two owned instances with different profiles and an unrelated daily instance
    When I stop one owned instance
    Then the other owned instance and daily instance remain alive

  Scenario: Reject a lost ownership token
    Given an instance token from an executor that has restarted
    When I request cleanup using that token
    Then cleanup fails without terminating any process by name or reused PID
