Feature: Proxy support
  As a developer routing traffic through proxies
  I want hpxless to use proxies for all requests
  So that I can scrape through different IP addresses

  Scenario: Navigate through SOCKS5 proxy
    Given a SOCKS5 proxy is available on port 1080
    And hpxless is started with port 0 and proxy "socks5://127.0.0.1:1080"
    When I connect to the CDP WebSocket endpoint
    And I navigate to "data:text/html,<p>proxied</p>"
    And I send Runtime.evaluate with expression "document.querySelector('p').textContent"
    Then the result is "proxied"
