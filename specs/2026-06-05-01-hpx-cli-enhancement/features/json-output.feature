Feature: hpx-cli JSON Output
  As a programmatic consumer of hpx-cli
  I want structured JSON output
  So I can parse results in scripts

  Scenario: Download list in JSON format
    Given the download engine has downloads
    When I run "hpx dl list --format json"
    Then the output is valid JSON
    And the JSON is an array of download objects
    And each object contains id, url, state, and progress fields

  Scenario: Download status in JSON format
    Given the download engine has a download with id "test-id"
    When I run "hpx dl status test-id --format json"
    Then the output is valid JSON
    And the JSON contains id, url, state, bytes_downloaded, and total_bytes fields
