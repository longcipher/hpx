Feature: Resource blocking
  As a developer using hpxless
  I want to block certain resource types
  So that I can speed up page loads and reduce bandwidth

  @skip
  Scenario: Block image requests
    Given hpxless is started with url "data:text/html,<img src='https://example.com/img.png'>"
    When I connect to the CDP WebSocket endpoint
    And I enable the Network domain
    And I set blocked URL patterns to ["*.png", "*.jpg"]
    And I send Page.navigate to "https://example.com"
    Then no requests matching "*.png" were made

  @skip
  Scenario: Block media requests
    Given hpxless is started with url "data:text/html,<video src='https://example.com/vid.mp4'></video>"
    When I connect to the CDP WebSocket endpoint
    And I enable the Network domain
    And I set blocked URL patterns to ["*.mp4", "*.webm"]
    And I send Page.navigate to "https://example.com"
    Then no requests matching "*.mp4" were made
