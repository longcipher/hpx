Feature: hpx-cli Download Progress Display
  As a user downloading files with hpx
  I want to see real-time progress
  So I know how long the download will take

  Scenario: Download with progress display
    Given the download engine is running
    And a file is available at "https://httpbin.org/bytes/1048576"
    When I run "hpx dl add https://httpbin.org/bytes/1048576"
    Then the download is added successfully
    And progress is displayed on stderr
    And the download completes

  Scenario: Download progress not shown in silent mode
    Given the download engine is running
    And a file is available at "https://httpbin.org/bytes/1048576"
    When I run "hpx dl add https://httpbin.org/bytes/1048576 --silent"
    Then the download is added successfully
    And no progress is displayed on stderr
