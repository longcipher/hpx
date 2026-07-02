Feature: HTTP client API unification
  As a developer working on hpx-browser
  I want a single unified request method on HttpClient instead of ten near-duplicate methods
  So that adding new HTTP verbs or behaviors requires changes in exactly one place

  Background:
    Given an hpx-browser HttpClient with cookie jar and browser profile configured

  Scenario: Unified request method handles GET
    When I call `client.request(Method::GET, url)` with no body
    Then the response status is 200
    And the cookie jar has captured any Set-Cookie headers

  Scenario: Unified request method handles POST with body
    When I call `client.request(Method::POST, url, Some(body_bytes))` 
    Then the response status is 200
    And the request body was transmitted correctly

  Scenario: Redirect following is policy-based
    When I navigate to a URL that returns 302 with a Location header
    And the redirect policy is Follow(max_redirects=5)
    Then the final response URL matches the Location header
    And cookies set during the redirect chain are captured

  Scenario: Manual redirect is still possible
    When the redirect policy is Manual
    And the server returns a 302 response
    Then I receive the raw 302 response without following

  Scenario: All existing public methods still work
    When I call `client.get(url)`, `client.post(url, body)`, `client.get_follow(url, n)`
    Then each behaves identically to the current implementation

  Scenario: Cookie injection happens for all request methods
    When I inject cookies via `client.inject_cookies(url, cookies)` 
    And then make any type of HTTP request to that URL
    Then the Cookie header is included in the outgoing request
