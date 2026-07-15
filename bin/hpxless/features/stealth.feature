Feature: Stealth profile
  As a developer using hpxless
  I want stealth mode to hide browser automation fingerprints
  So that I can avoid bot detection

  @wip
  Scenario: Stealth mode starts successfully
    Given hpxless is started with stealth enabled
    When I connect to the CDP WebSocket endpoint
    And I send Browser.getVersion
    Then I receive a valid version response

  @skip
  Scenario: Stealth mode patches navigator.webdriver
    Given hpxless is started with stealth enabled
    When I connect to the CDP WebSocket endpoint
    And I send Runtime.evaluate with expression "navigator.webdriver"
    Then the result is "undefined"

  @skip
  Scenario: Stealth mode uses consistent user agent
    Given hpxless is started with stealth enabled
    When I connect to the CDP WebSocket endpoint
    And I send Runtime.evaluate with expression "navigator.userAgent"
    Then the result contains "Mozilla"
