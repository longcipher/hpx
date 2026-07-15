Feature: Resource blocking
  As a developer scraping text content
  I want to block images and media during navigation
  So that I save bandwidth and memory

  Scenario: Block images during navigation
    Given hpxless is started with port 0 and block "images"
    When I connect to the CDP WebSocket endpoint
    And I navigate to "data:text/html,<img src='http://example.com/image.png'>"
    Then no request is made to "http://example.com/image.png"

  Scenario: Block media during navigation
    Given hpxless is started with port 0 and block "media"
    When I connect to the CDP WebSocket endpoint
    And I navigate to "data:text/html,<video src='http://example.com/video.mp4'></video>"
    Then no request is made to "http://example.com/video.mp4"

  Scenario: Allow non-blocked resources
    Given hpxless is started with port 0 and block "images"
    When I connect to the CDP WebSocket endpoint
    And I navigate to "data:text/html,<script>window.ok=true</script>"
    And I send Runtime.evaluate with expression "window.ok"
    Then the result is "true"
