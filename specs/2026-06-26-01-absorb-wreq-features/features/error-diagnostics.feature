Feature: Error Source Chain Diagnostics
  As an HTTP client user
  I want accurate error classification even through tower layers
  So that I can correctly identify timeout, connect, and DNS errors

  Scenario: Direct timeout error is detected
    Given an Error created from a TimedOut source
    Then is_timeout() should return true

  Scenario: Timeout error wrapped in tower elapsed is detected
    Given an Error created from a tower::timeout::error::Elapsed wrapping a TimedOut
    Then is_timeout() should return true

  Scenario: Timeout error nested three levels deep is detected
    Given an Error with a 3-level source chain containing TimedOut at level 3
    Then is_timeout() should return true

  Scenario: io::ErrorKind::TimedOut is detected as timeout
    Given an Error wrapping an io::Error with kind TimedOut
    Then is_timeout() should return true

  Scenario: Non-timeout error returns false
    Given an Error created from a generic message
    Then is_timeout() should return false

  Scenario: Connect error through tower layers is detected
    Given an Error wrapping a connect error through service middleware
    Then is_connect() should return true

  Scenario: DNS error in source chain is detected
    Given an Error wrapping a DNS resolution failure
    Then is_dns() should return true
