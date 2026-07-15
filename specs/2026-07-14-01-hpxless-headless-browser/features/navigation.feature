Feature: Page navigation with sub-resource loading
  As a developer automating web pages
  I want pages to fully load with CSS, JS, and images
  So that I can interact with the rendered page

  Scenario: Navigate to a page with sub-resources
    Given hpxless is started with port 0
    When I connect to the CDP WebSocket endpoint
    And I navigate to "data:text/html,<link rel='stylesheet' href='data:text/css,h1{color:rgb(255,0,0)}'><h1>styled</h1>"
    And I send Runtime.evaluate with expression "getComputedStyle(document.querySelector('h1')).color"
    Then the result contains "255, 0, 0"

  Scenario: Execute inline scripts during load
    Given hpxless is started with port 0
    When I connect to the CDP WebSocket endpoint
    And I navigate to "data:text/html,<script>window.x=42</script><p>test</p>"
    And I send Runtime.evaluate with expression "window.x"
    Then the result is "42"

  Scenario: Execute multiple inline scripts in order
    Given hpxless is started with port 0
    When I connect to the CDP WebSocket endpoint
    And I navigate to "data:text/html,<script>window.order=[];window.order.push(1)</script><script>window.order.push(2)</script>"
    And I send Runtime.evaluate with expression "window.order.join(',')"
    Then the result is "1,2"

  Scenario: Apply external CSS stylesheet
    Given a local HTTP server serves "/style.css" with content "h1{font-size:42px}"
    And hpxless is started with port 0
    When I connect to the CDP WebSocket endpoint
    And I navigate to the page linking "/style.css"
    And I send Runtime.evaluate with expression "getComputedStyle(document.querySelector('h1')).fontSize"
    Then the result is "42px"

  Scenario: Execute external script
    Given a local HTTP server serves "/app.js" with content "window.loaded=true"
    And hpxless is started with port 0
    When I connect to the CDP WebSocket endpoint
    And I navigate to the page including "/app.js"
    And I send Runtime.evaluate with expression "window.loaded"
    Then the result is "true"

  Scenario: Layout computed only on demand
    Given hpxless is started with port 0
    When I connect to the CDP WebSocket endpoint
    And I navigate to "data:text/html,<div style='width:100px;height:50px'></div>"
    And I send Runtime.evaluate with expression "document.querySelector('div').getBoundingClientRect().width"
    Then the result is "100"
