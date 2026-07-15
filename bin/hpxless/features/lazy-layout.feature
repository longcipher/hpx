Feature: Lazy layout (layout-on-demand)
  As a developer using hpxless
  I want layout to be computed lazily
  So that pages without layout queries skip layout overhead

  @wip
  Scenario: Read element text without triggering layout
    Given hpxless is started with url "data:text/html,<div id=d>hello</div>"
    When I connect to the CDP WebSocket endpoint
    And I send Runtime.evaluate with expression "document.getElementById('d').textContent"
    Then the result is "hello"

  @skip
  Scenario: Query computed style triggers layout
    Given hpxless is started with url "data:text/html,<style>body{font-size:20px}</style><div id=d>text</div>"
    When I connect to the CDP WebSocket endpoint
    And I send Runtime.evaluate with expression "getComputedStyle(document.body).fontSize"
    Then the result is "20px"

  @skip
  Scenario: Layout is deferred until getComputedStyle is called
    Given hpxless is started with url "data:text/html,<div>lazy</div>"
    When I connect to the CDP WebSocket endpoint
    And I check layout has not been computed
    And I send Runtime.evaluate with expression "getComputedStyle(document.body).display"
    Then layout was computed exactly once
