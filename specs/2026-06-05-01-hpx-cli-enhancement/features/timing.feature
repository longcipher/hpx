Feature: hpx-cli Timing Waterfall
  As a user debugging HTTP requests
  I want to see detailed timing breakdown
  So I can identify performance bottlenecks

  Scenario: Request with timing waterfall
    Given the HTTP client is available
    When I run "hpx -T https://httpbin.org/get"
    Then the response is displayed
    And the timing waterfall shows DNS resolution time
    And the timing waterfall shows TCP connect time
    And the timing waterfall shows TLS handshake time
    And the timing waterfall shows time to first byte
    And the timing waterfall shows transfer time
    And the timing waterfall shows total time

  Scenario: Request without timing flag
    Given the HTTP client is available
    When I run "hpx https://httpbin.org/get"
    Then the response is displayed
    And no timing waterfall is shown
