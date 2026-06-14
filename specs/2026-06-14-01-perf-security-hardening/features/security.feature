Feature: Security hardening for hpx workspace
  As a developer using hpx in production
  I want the HTTP client to be resistant to injection and credential leaks
  So that sensitive data is not exposed through logs or request smuggling

  # Finding 9: CRLF injection in WS HTTP CONNECT tunnel
  @finding-9 @security @S
  Scenario: HTTP CONNECT tunnel rejects hosts with control characters
    Given a WebSocket connection through an HTTP proxy
    When the target host is validated for the CONNECT request
    Then only valid hostname characters are accepted (alphanumeric, dots, hyphens, colons, brackets)
    And hosts containing CR or LF characters are rejected with an error

  # Finding 10: LoggingHook leaks Authorization headers
  @finding-10 @security @S
  Scenario: Sensitive headers are redacted in request logging
    Given a LoggingHook with log_headers enabled
    When request headers are logged at TRACE level
    Then Authorization, Cookie, and Proxy-Authorization header values are redacted
    And the header name is still logged but the value shows "[REDACTED]"

  # Finding 11: Proxy credentials logged in TRACE
  @finding-11 @security @S
  Scenario: Proxy credentials are not logged in trace output
    Given a proxy connection with credentials in the URI
    When the proxy URI is logged at TRACE level
    Then the credentials portion of the URI is masked
    And only the proxy host and port are visible in the log
