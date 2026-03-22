Feature: Progress Reporting
  As a developer using hpx-dl
  I want to receive progress events during downloads
  So that I can display progress to users

  Scenario: Subscriber receives all download events
    Given a file "events.bin" of 1MB is available at "http://localhost:8080/events.bin"
    And an hpx-dl engine is configured
    When I subscribe to download events
    And I add a download for "http://localhost:8080/events.bin" to "events.bin"
    And I wait for the download to complete
    Then I should have received an "Added" event
    And I should have received a "Started" event
    And I should have received at least 1 "Progress" event
    And I should have received a "Completed" event
