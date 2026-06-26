Feature: Connection Pool Isolation
  As an HTTP client user with multiple emulation profiles
  I want connections isolated by emulation profile
  So that different fingerprints don't share connections

  Scenario: Same host same emulation shares pool entry
    Given two requests to "httpbin.org" both using Chrome147 emulation
    When both requests complete
    Then they should share the same pool entry

  Scenario: Same host different emulations get separate pool entries
    Given a request to "httpbin.org" using Chrome147 emulation
    And another request to "httpbin.org" using Firefox136 emulation
    When both requests complete
    Then they should have separate pool entries

  Scenario: Default emulation uses host-only pool key
    Given two requests to "httpbin.org" with no per-request emulation
    When both requests complete
    Then they should share the same pool entry

  Scenario: forbid_recycle prevents connection reuse
    Given a successful request to "httpbin.org"
    When I call forbid_recycle() on the response
    And I make another request to "httpbin.org"
    Then a new connection should be established
    And the previous connection should not be reused
