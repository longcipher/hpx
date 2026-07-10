Feature: Test coverage — Add unit tests for critical untested modules

  Background:
    Given a working hpx workspace with all crates compiled

  # --- Cookie Module Tests ---

  @finding-TEST-05 @testing @priority-high
  Scenario: Cookie jar stores and retrieves cookies by domain
    When set_cookies is called with Set-Cookie headers for a URL
    Then cookies for that URL SHALL be retrievable via the cookies method
    And cookies SHALL NOT be returned for unrelated domains

  @finding-TEST-05 @testing @priority-high
  Scenario: Cookie jar respects path matching
    Given a cookie is set with path=/app
    When cookies are requested for a URL under /app
    Then the cookie SHALL be returned
    But when cookies are requested for a URL under /other
    Then the cookie SHALL NOT be returned

  @finding-TEST-05 @testing @priority-high
  Scenario: Cookie jar handles concurrent access
    Given a shared Jar instance
    When multiple tasks insert and read cookies concurrently
    Then no data races or panics SHALL occur
    And all inserted cookies SHALL be readable

  @finding-TEST-05 @testing @priority-medium
  Scenario: Cookie jar handles expired cookies
    Given a cookie is set with an Expires attribute in the past
    When cookies are requested for that domain
    Then the expired cookie SHALL NOT be returned

  @finding-TEST-05 @testing @priority-medium
  Scenario: Cookie jar handles secure and HttpOnly attributes
    Given a cookie is set with Secure and HttpOnly attributes
    Then the parsed cookie SHALL correctly reflect both attributes

  # --- Retry Module Tests ---

  @finding-TEST-06 @testing @priority-high
  Scenario: Retry policy scope excludes non-idempotent methods by default
    Given a RetryPolicy with default scope
    When a POST request fails with a transient error
    Then the policy SHALL NOT retry the request

  @finding-TEST-06 @testing @priority-high
  Scenario: Retry budget is consumed only on actual retries
    Given a RetryPolicy with budget N
    When N retries are performed
    Then the budget SHALL be exhausted
    And subsequent transient errors SHALL NOT trigger retries

  @finding-TEST-06 @testing @priority-medium
  Scenario: Retry policy classifier chains work correctly
    Given a RetryPolicy with multiple classifiers
    When a response status matches one classifier but not another
    Then the combined classification SHALL be the union of all matching classifiers

  # --- Redirect Module Tests ---

  @finding-TEST-07 @testing @priority-high
  Scenario: Redirect strips Authorization header on cross-origin redirect
    Given a redirect from https://a.com to https://b.com
    When the redirect policy processes sensitive headers
    Then the Authorization header SHALL be stripped
    And the User-Agent header SHALL be preserved

  @finding-TEST-07 @testing @priority-high
  Scenario: Redirect preserves Authorization on same-origin redirect
    Given a redirect from https://a.com/path1 to https://a.com/path2
    When the redirect policy processes sensitive headers
    Then the Authorization header SHALL be preserved

  @finding-TEST-07 @testing @priority-medium
  Scenario: Redirect loop detection stops after max redirects
    Given a redirect policy with max redirects set to 5
    When 6 consecutive redirects occur
    Then the redirect SHALL fail with a loop detected error

  # --- Proxy Module Tests ---

  @finding-TEST-08 @testing @priority-high
  Scenario: Proxy rule matches URL against proxy patterns
    Given a proxy with rules for *.example.com
    When a request is made to api.example.com
    Then the proxy SHALL be used
    But when a request is made to other.com
    Then the proxy SHALL NOT be used

  @finding-TEST-08 @testing @priority-medium
  Scenario: Custom HTTP auth is applied to proxy requests
    Given a proxy configured with custom HTTP auth
    When the proxy URL is constructed for a target URL
    Then the Proxy-Authorization header SHALL be set correctly
