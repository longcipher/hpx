Feature: Alt-Svc Discovery and HTTP/3 Fallback
  As a high-performance HTTP client
  I want to discover HTTP/3 endpoints via the Alt-Svc header (RFC 7838)
  And gracefully fall back to HTTP/2 when QUIC is unreachable
  So that I can use HTTP/3 when available without sacrificing reliability

  Background:
    Given the `http3` Cargo feature is enabled
    And a local h2 test server is running on `127.0.0.1:8443`
    And a local h3 test server is running on `127.0.0.1:4433`

  # REQ-05, REQ-09, REQ-16, REQ-19
  # C-09, C-10, C-11, C-12, C-13, C-14, C-18

  Scenario: Alt-Svc header is parsed
    Given an `Alt-Svc` header value `h3=":4433"; ma=3600`
    When I parse the header with `parse_alt_svc`
    Then the parser returns one entry with `protocol_id = "h3"`, `alt_authority = ":4433"`, `max_age = 3600`

  Scenario: Alt-Svc clear directive invalidates cache
    Given an `Alt-Svc` header value `clear`
    When I parse the header with `parse_alt_svc`
    Then the parser returns a `clear` directive
    And applying it to the cache invalidates all entries for the authority

  Scenario: Alt-Svc malformed entry is rejected
    Given an `Alt-Svc` header value `h3=":4433"; ma=abc`
    When I parse the header with `parse_alt_svc`
    Then the parser rejects the malformed entry
    And no entry is inserted into the cache

  Scenario: Alt-Svc multiple entries parsed
    Given an `Alt-Svc` header value `h3=":4433"; ma=3600, h3-29=":4434"; ma=60`
    When I parse the header with `parse_alt_svc`
    Then the parser returns two entries
    And the first entry has `protocol_id = "h3"` and `alt_authority = ":4433"`
    And the second entry has `protocol_id = "h3-29"` and `alt_authority = ":4434"`

  Scenario: Alt-Svc cache entry expires after TTL
    Given an `Alt-Svc` cache entry for `127.0.0.1:8443` with `ma=1`
    When 2 seconds elapse
    And I look up the entry for `127.0.0.1:8443`
    Then the lookup returns `None` (expired)

  Scenario: Alt-Svc header is captured from h2 response
    Given a `Client` built with `prefer_http3()`
    When I send a GET to `https://127.0.0.1:8443/` and the server responds with `Alt-Svc: h3=":4433"; ma=3600`
    Then the `AltSvcCache` contains an entry for `127.0.0.1:8443` pointing to `:4433` over `h3`

  Scenario: Client upgrades to h3 after receiving Alt-Svc
    Given a `Client` built with `prefer_http3()`
    And the first GET to `https://127.0.0.1:8443/` returned `Alt-Svc: h3=":4433"; ma=3600`
    When I send a second GET to `https://127.0.0.1:8443/`
    Then the second request is sent over HTTP/3 to `127.0.0.1:4433`
    And the response version is `Version::HTTP_3`

  Scenario: prefer_http3() prefers h3 with fallback to h2
    Given a `Client` built with `prefer_http3()`
    And an `Alt-Svc` cache entry exists for `127.0.0.1:8443` pointing to h3 on `:4433`
    When the h3 server on `:4433` is unreachable (stopped)
    And I send a GET to `https://127.0.0.1:8443/`
    Then the client falls back to HTTP/2 within 5 seconds
    And the response version is `Version::HTTP_2`
    And the `AltSvcCache` entry is NOT cleared (transient failure)

  Scenario: QUIC unreachable triggers fallback to h2
    Given a `Client` built with `prefer_http3()`
    And the h3 server on `:4433` has been unreachable for 3 consecutive attempts
    When I send a GET to `https://127.0.0.1:8443/`
    Then the client skips the h3 attempt (circuit breaker open)
    And the request is sent directly over HTTP/2
    And the circuit breaker cooldown is 60 seconds

  Scenario: Alt-Svc cache entry survives transient h3 failure
    Given a `Client` built with `prefer_http3()`
    And an `Alt-Svc` cache entry exists for `127.0.0.1:8443`
    When the h3 server briefly fails (1 attempt) and then recovers
    And I send a subsequent GET
    Then the client retries h3 after the brief failure
    And the response version is `Version::HTTP_3`
