Feature: Speed Limiting
  As a developer using hpx-dl
  I want to limit download speed
  So that downloads don't consume all available bandwidth

  Scenario: Global speed limit is enforced
    Given a file "throttled.bin" of 2MB is available at "http://localhost:8080/throttled.bin"
    And the server supports range requests
    And an hpx-dl engine is configured with global speed limit 512KB/s
    When I add a download for "http://localhost:8080/throttled.bin" to "throttled.bin"
    And I wait for the download to complete
    Then the download state should be "Completed"
    And the elapsed time should be at least 3 seconds
    And the average speed should not exceed 600KB/s
