Feature: Network Request Interception
  As a user of hpx-browser
  I want to intercept and modify network requests
  So that I can block ads, mock responses, or modify headers

  Scenario: Intercept and allow requests
    Given a CdpPage with network interception enabled
    When I set a callback that allows all requests
    And I navigate to "https://example.com"
    Then the page should load successfully
    And the callback should have been called

  Scenario: Intercept and abort specific requests
    Given a CdpPage with network interception enabled
    When I set a callback that blocks requests to "analytics.example.com"
    And I navigate to "https://example.com"
    Then the page should load
    And requests to "analytics.example.com" should be blocked

  Scenario: Callback receives URL and resource type
    Given a CdpPage with network interception enabled
    When I set a callback that records all requests
    And I navigate to "https://example.com"
    Then the callback should receive the request URL
    And the callback should receive the resource type

  Scenario: Interception is session-scoped
    Given a Browser with 2 pages
    When I enable interception on page 1 only
    And both pages navigate
    Then only page 1 should have interception active
