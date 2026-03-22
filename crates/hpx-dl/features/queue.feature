Feature: Download Queue Priority
  As a developer using hpx-dl
  I want downloads to be prioritized
  So that important files download first

  Scenario: Critical priority downloads before Normal
    Given an hpx-dl engine is configured with max 1 concurrent download
    And a file "normal.bin" of 1MB is available at "http://localhost:8080/normal.bin"
    And a file "critical.bin" of 1MB is available at "http://localhost:8080/critical.bin"
    When I add a download for "http://localhost:8080/normal.bin" to "normal.bin" with priority "Normal"
    And I add a download for "http://localhost:8080/critical.bin" to "critical.bin" with priority "Critical"
    And I wait for all downloads to complete
    Then the "critical.bin" download should have completed before the "normal.bin" download
