Feature: JavaScript Execution
  As an hpx-browser user
  I want to execute JavaScript in the page context
  So that I can interact with JS-heavy pages

  Scenario: Evaluate simple expression
    Given a page with an empty body
    When I evaluate "1 + 2"
    Then the result should be "3"

  Scenario: DOM manipulation via JS
    Given a page with "<div id='target'>Old</div>"
    When I evaluate "document.getElementById('target').textContent = 'New'"
    Then the text of "#target" should be "New"

  Scenario: Timer execution
    Given a page with an empty body
    When I evaluate "setTimeout(() => { document.title = 'Timer Done' }, 100)"
    And I wait for the event loop to drain
    Then the page title should be "Timer Done"

  Scenario: Fetch API
    Given a page with an empty body
    When I evaluate async "fetch('https://httpbin.org/get').then(r => r.json()).then(d => document.title = d.url)"
    Then the page title should contain "httpbin.org"
