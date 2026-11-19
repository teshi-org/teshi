Feature: Login
  The demo runner emits screenshots, HTTP exchanges, logs, recordings and
  unknown artifacts so every Run Details family can be exercised.

  Scenario: Successful login
    Given the API is running
    When I POST /api/login
    Then status is 200
    And a session cookie is set

  Scenario: Rejected password
    Given the API is running
    When I POST /api/login with a wrong password
    Then status is 200
    And a session cookie is set

  Scenario: Locked account
    Given the account is locked
    When I try to log in
    Then the response is blocked

  Scenario: Unavailable target
    Given the browser session is unavailable
    When I try to log in
    Then the scenario cannot be judged
