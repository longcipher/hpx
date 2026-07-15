Feature: Proxy navigation
  As a developer using hpxless
  I want to route traffic through a proxy
  So that I can access pages behind network restrictions

  @skip
  Scenario: Navigate through HTTP proxy
    Given hpxless is started with proxy "http://localhost:8080"
    When I connect to the CDP WebSocket endpoint
    And I enable the Page domain
    And I send Page.navigate to "https://example.com"
    Then I receive "Page.loadEventFired" event

  @skip
  Scenario: Navigate through SOCKS5 proxy
    Given hpxless is started with proxy "socks5://localhost:1080"
    When I connect to the CDP WebSocket endpoint
    And I enable the Page domain
    And I send Page.navigate to "https://example.com"
    Then I receive "Page.loadEventFired" event
