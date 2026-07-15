Feature: Stealth anti-detection
  As a developer avoiding bot detection
  I want stealth profiles active during browsing
  So that the browser appears as a real user

  Scenario: Stealth profile active
    Given hpxless is started with port 0 and stealth enabled
    When I connect to the CDP WebSocket endpoint
    And I navigate to "data:text/html,<p>test</p>"
    And I send Runtime.evaluate with expression "navigator.webdriver"
    Then the result is "false"

  Scenario: Chrome object present with stealth
    Given hpxless is started with port 0 and stealth enabled
    When I connect to the CDP WebSocket endpoint
    And I navigate to "data:text/html,<p>test</p>"
    And I send Runtime.evaluate with expression "typeof window.chrome"
    Then the result is "object"
