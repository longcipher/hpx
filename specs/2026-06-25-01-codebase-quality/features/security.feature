Feature: Security Hardening

  Fix credential leaks, injection vulnerabilities, and access control gaps
  in the CLI and library.

  @finding-SEC-002 @priority-medium
  Scenario: All Set-Cookie headers are saved to cookie jar
    Given an HTTP client with --cookie-jar set to /tmp/cookies.txt
    And a server responds with 3 Set-Cookie headers
    When the response is processed
    Then all 3 cookies should be written to /tmp/cookies.txt
    And each cookie should be on its own line

  @finding-SEC-002 @priority-medium
  Scenario: Cookie jar append mode preserves existing cookies
    Given an HTTP client with --cookie-jar set to /tmp/cookies.txt
    And the jar file already contains 2 cookies
    When a response arrives with 1 new Set-Cookie header
    Then the jar file should contain 3 cookies total
    And the existing cookies should not be overwritten

  @finding-SEC-003 @priority-medium
  Scenario: NDJSON output escapes special characters in URLs
    Given the hpx fetch --dump assets command
    And a page with an asset URL containing double quotes
    When the assets are formatted as NDJSON
    Then the output should be valid JSON on each line
    And the URL value should be properly escaped

  @finding-SEC-003 @priority-medium
  Scenario: NDJSON output handles normal URLs correctly
    Given the hpx fetch --dump assets command
    And a page with normal asset URLs
    When the assets are formatted as NDJSON
    Then each line should be valid JSON
    And the url and type fields should match the original values

  @finding-SEC-004 @priority-low
  Scenario: Proxy test respects certificate verification by default
    Given the hpx proxy-test command
    When testing an HTTPS proxy endpoint
    Then certificate verification should be enabled
    And the test should fail if the certificate is invalid

  @finding-SEC-009 @priority-high
  Scenario: CDP serve warns when binding to non-loopback address
    Given the hpx serve --host 0.0.0.0 command
    When the CDP WebSocket server starts
    Then a warning should be printed about external access
    And the warning should mention the security implications

  @finding-SEC-009 @priority-high
  Scenario: CDP serve defaults to loopback
    Given the hpx serve command with default host
    When the CDP WebSocket server starts
    Then it should bind to 127.0.0.1
    And no external access warning is needed

  @finding-SEC-011 @priority-medium
  Scenario: Proxy credentials are stripped before persistence
    Given a download request with proxy URL http://user:pass@proxy:8080
    When the download is persisted to SQLite storage
    Then the stored proxy URL should be http://proxy:8080
    And the credentials should not appear in the database
