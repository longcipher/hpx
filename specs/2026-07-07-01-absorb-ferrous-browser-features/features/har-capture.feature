Feature: HAR Capture
  As a user of hpx-browser
  I want to capture network traffic in HAR format
  So that I can analyze and replay HTTP interactions

  Scenario: Start and stop HAR capture
    Given a CdpPage connected to Chrome
    When I start HAR capture
    And I navigate to "https://example.com"
    And I stop HAR capture
    Then the HAR archive should contain at least one entry
    And the entry should have a request with URL "https://example.com"
    And the entry should have a response with status 200

  Scenario: HAR snapshot without stopping
    Given a CdpPage with active HAR capture
    When I navigate to "https://example.com"
    And I call export() to snapshot
    Then the snapshot should contain the request
    And the capture should still be active

  Scenario: HAR captures request headers
    Given a CdpPage with HAR capture
    When I navigate to "https://example.com"
    And I stop HAR capture
    Then the HAR entry should have request headers
    And the User-Agent header should be present

  Scenario: HAR captures response headers
    Given a CdpPage with HAR capture
    When I navigate to "https://example.com"
    And I stop HAR capture
    Then the HAR entry should have response headers
    And the Content-Type header should be present

  Scenario: HAR captures timings
    Given a CdpPage with HAR capture
    When I navigate to "https://example.com"
    And I stop HAR capture
    Then the HAR entry should have timing information
    And the wait time should be non-negative

  Scenario: Clear HAR state
    Given a CdpPage with HAR capture that has entries
    When I call clear()
    Then the pending requests should be empty
    And the entries should be empty

  Scenario: ISO 8601 timestamp formatting
    When I format timestamp 0 as ISO 8601
    Then the result should be "1970-01-01T00:00:00.000Z"

  Scenario: ISO 8601 handles recent timestamps
    When I format timestamp 1700000000 as ISO 8601
    Then the result should match the pattern "2023-11-14T22:13:20.000Z"
