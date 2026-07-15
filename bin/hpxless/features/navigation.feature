Feature: Navigation pipeline
  As a developer using hpxless via CDP
  I want to navigate to pages and inspect the DOM
  So that I can automate page interactions

  @skip
  Scenario: Navigate to data URL and read inline script globals
    Given hpxless is started with url "data:text/html,<script>window.x = 42;</script>"
    When I connect to the CDP WebSocket endpoint
    And I enable the Page domain
    And I execute inline scripts
    And I send Runtime.evaluate with expression "window.x"
    Then the result is "42"

  @wip
  Scenario: Navigate to data URL and read inline CSS computed style
    Given hpxless is started with url "data:text/html,<style>body{color:red}</style>"
    When I connect to the CDP WebSocket endpoint
    And I send Runtime.evaluate with expression "getComputedStyle(document.body).color"
    Then the result is "red"

  @wip
  Scenario: Navigate via Page.navigate to about:blank
    Given hpxless is started with port 0
    When I connect to the CDP WebSocket endpoint
    And I enable the Page domain
    And I send Page.navigate to "about:blank"
    Then the response contains "frameId"
    And the response contains "loaderId"

  @wip
  Scenario: Navigate via Page.navigate and receive lifecycle events
    Given hpxless is started with port 0
    When I connect to the CDP WebSocket endpoint
    And I enable the Page domain
    And I send Page.navigate to "about:blank"
    Then I receive "Page.frameNavigated" event
    And I receive "Page.domContentEventFired" event
    And I receive "Page.loadEventFired" event

  @wip
  Scenario: Navigate with Network domain enabled emits network events
    Given hpxless is started with port 0
    When I connect to the CDP WebSocket endpoint
    And I enable the Page domain
    And I enable the Network domain
    And I send Page.navigate to "about:blank"
    Then I receive "Network.requestWillBeSent" event
    And I receive "Network.responseReceived" event
    And I receive "Network.loadingFinished" event

  @wip
  Scenario: DOM.getDocument returns document node
    Given hpxless is started with url "data:text/html,<p>hello</p>"
    When I connect to the CDP WebSocket endpoint
    And I send DOM.getDocument
    Then the response contains "nodeType"
    And the response contains "#document"

  @skip
  Scenario: Navigate to external URL and load sub-resources
    Given hpxless is started with port 0
    When I connect to the CDP WebSocket endpoint
    And I enable the Page domain
    And I enable the Network domain
    And I send Page.navigate to "https://example.com"
    Then I receive "Network.requestWillBeSent" event
    And I receive "Page.loadEventFired" event
