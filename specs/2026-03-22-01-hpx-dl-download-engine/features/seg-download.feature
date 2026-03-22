Feature: Segmented Download
  As a developer using hpx-dl
  I want to download files using multiple connections
  So that downloads complete faster and are more reliable

  Scenario: Basic segmented download
    Given a file "large.bin" of 10MB is available at "http://localhost:8080/large.bin"
    And the server supports range requests
    And an hpx-dl engine is configured with max 4 connections per download
    When I add a download for "http://localhost:8080/large.bin" to "large.bin"
    And I wait for the download to complete
    Then the download state should be "Completed"
    And the file "large.bin" should be exactly 10485760 bytes
    And the file "large.bin" should match the original content

  Scenario: Resume interrupted download
    Given a file "resume.bin" of 5MB is available at "http://localhost:8080/resume.bin"
    And the server supports range requests
    And the server returns ETag "abc123" for "http://localhost:8080/resume.bin"
    And an hpx-dl engine is configured with max 2 connections per download
    When I add a download for "http://localhost:8080/resume.bin" to "resume.bin"
    And the download reaches 50% progress
    And I stop the engine
    And I create a new engine with the same storage path
    And I resume the download
    And I wait for the download to complete
    Then the download state should be "Completed"
    And the file "resume.bin" should be exactly 5242880 bytes
    And the file "resume.bin" should match the original content

  Scenario: Download with no range support falls back to single connection
    Given a file "small.txt" of 100KB is available at "http://localhost:8080/small.txt"
    And the server does not support range requests
    And an hpx-dl engine is configured with max 4 connections per download
    When I add a download for "http://localhost:8080/small.txt" to "small.txt"
    And I wait for the download to complete
    Then the download state should be "Completed"
    And the file "small.txt" should be exactly 102400 bytes
