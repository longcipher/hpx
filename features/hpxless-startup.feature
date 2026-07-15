Feature: hpxless startup and CDP server

  Scenario: hpxless starts and serves CDP
    Given hpxless is started with port 0
    When I connect to the CDP WebSocket endpoint
    And I send Browser.getVersion
    Then I receive a valid version response
    And the browser name contains "hpx"

  Scenario: hpxless starts with initial URL
    Given hpxless is started with url "data:text/html,<h1>hello</h1>"
    When I connect to the CDP WebSocket endpoint
    And I send Runtime.evaluate with expression "document.querySelector('h1').textContent"
    Then the result is "hello"

  Scenario: hpxless shuts down gracefully
    Given hpxless is started with port 0
    When I send SIGTERM to the process
    Then the process exits within 5 seconds
    And the exit code is 0
