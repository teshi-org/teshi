@api
Feature: User HTTP API

  Scenario: Create a user then fetch by extracted id
    When [API] I create a user named "Ada"
    Then [API] I fetch that user
