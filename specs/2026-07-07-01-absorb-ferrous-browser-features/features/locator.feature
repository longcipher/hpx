Feature: Locator API
  As a user of hpx-browser
  I want to interact with page elements using a Playwright-style Locator
  So that I can automate element interactions naturally

  Scenario: Click an element
    Given a CdpPage with a button element "#btn"
    When I create a Locator for "#btn"
    And I call click()
    Then the button should be clicked

  Scenario: Type text into an input
    Given a CdpPage with an input element "#name"
    When I create a Locator for "#name"
    And I type "hello world"
    Then the input should contain "hello world"
    And keyboard events should be dispatched for each character

  Scenario: Get element inner text
    Given a CdpPage with a div "#content" containing "Hello"
    When I create a Locator for "#content"
    And I call inner_text()
    Then the result should be "Hello"

  Scenario: Get element attribute
    Given a CdpPage with an element "#link" with href "https://example.com"
    When I create a Locator for "#link"
    And I call get_attribute("href")
    Then the result should be Some("https://example.com")

  Scenario: Wait for element to appear
    Given a CdpPage that will add "#dynamic" after 100ms
    When I create a Locator for "#dynamic"
    And I call wait_for(5s)
    Then the wait should succeed within 200ms

  Scenario: Wait for element timeout
    Given a CdpPage without element "#nonexistent"
    When I create a Locator for "#nonexistent"
    And I call wait_for(100ms)
    Then the wait should timeout
