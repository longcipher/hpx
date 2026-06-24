Feature: DOM Rendering
  As an hpx-browser user
  I want HTML pages parsed into a DOM tree
  So that I can query and interact with page elements

  Scenario: Parse simple HTML
    Given an HTML string "<html><head><title>Test</title></head><body><p>Hello</p></body></html>"
    When I parse the HTML into a DOM
    Then the DOM should have a document node
    And the title element should contain "Test"
    And the body should contain a paragraph with text "Hello"

  Scenario: Query elements by selector
    Given an HTML page with "<div class='content'><p id='first'>A</p><p>B</p></div>"
    When I query for "p" elements
    Then I should get 2 results
    When I query for "#first"
    Then I should get 1 result with text "A"
    When I query for ".content p"
    Then I should get 2 results

  Scenario: Mutate DOM
    Given an HTML page with "<div id='root'></div>"
    When I append a child paragraph with text "New" to "#root"
    Then "#root" should have 1 child
    And the child text should be "New"
