Feature: Checkout
  A second Feature shows that the details pane keeps the same structure when
  the selected runner case changes.

  Scenario: Successful checkout
    Given a cart with one item
    When I pay with a saved card
    Then the order is created

  Scenario: Delivery quote blocked
    Given the delivery service is offline
    When I request a quote
    Then the quote cannot be judged
