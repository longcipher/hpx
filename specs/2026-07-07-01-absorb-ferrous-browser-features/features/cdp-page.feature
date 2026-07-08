Feature: CdpPage Navigation and Interaction
  As a user of hpx-browser
  I want to navigate pages and interact with elements via CDP
  So that I can automate real browser workflows

  Scenario: Navigate to URL with Load wait
    Given a CdpPage connected to Chrome
    When I navigate to "https://example.com" with WaitUntil::Load
    Then the page should finish loading
    And the page URL should be "https://example.com"

  Scenario: Navigate with NetworkIdle wait
    Given a CdpPage connected to Chrome
    When I navigate to "https://example.com" with WaitUntil::NetworkIdle
    Then the page should wait for network idle
    And the page should be ready

  Scenario: Get page title
    Given a CdpPage on "https://example.com"
    When I call title()
    Then it should return "Example Domain"

  Scenario: Get page content
    Given a CdpPage on "https://example.com"
    When I call content()
    Then it should contain the HTML document
    And it should contain the body element

  Scenario: Evaluate JavaScript expression
    Given a CdpPage on "https://example.com"
    When I evaluate "1 + 2"
    Then the result should be 3

  Scenario: Evaluate typed JavaScript
    Given a CdpPage on "https://example.com"
    When I evaluate "document.title" as String
    Then the result should be the page title

  Scenario: Take screenshot
    Given a CdpPage on "https://example.com"
    When I take a screenshot
    Then the result should be valid PNG bytes
    And the PNG magic bytes should be present

  Scenario: Export as PDF
    Given a CdpPage on "https://example.com"
    When I export as PDF
    Then the result should be valid PDF bytes
    And the PDF header should be "%PDF-"
