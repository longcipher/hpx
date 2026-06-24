Feature: Page Navigation
  As an hpx-browser user
  I want to navigate to URLs and get rendered page content
  So that I can scrape JS-heavy pages without headless Chrome

  Scenario: Navigate to clean page
    Given a simple HTML page with title "Test Page" and body "Hello World"
    When I navigate to the page
    Then the page title should be "Test Page"
    And the page text content should contain "Hello World"
    And the challenge verdict should be Pass

  Scenario: Navigate with challenge detection
    Given a page that returns a Cloudflare challenge response
    When I navigate to the page
    Then the challenge verdict should be ChallengeIncomplete
    And the response body should contain challenge markers

  Scenario: Navigate warm reuse
    Given a page that has been navigated once
    When I navigate to a second URL using warm reuse
    Then the navigation should complete faster than a cold start
    And the page content should reflect the second URL
