Feature: Security hardening — SSRF prevention, TLS key log gating, path traversal prevention

  Background:
    Given a working hpx workspace with all crates compiled

  # --- SSRF Prevention ---

  @finding-SEC-013 @security @priority-high
  Scenario: Browser fetch blocks private IP addresses by default
    When a user runs "hpx fetch http://127.0.0.1" without --allow-private-network
    Then the request SHALL fail with an error indicating the address is forbidden
    And the error message SHALL mention that --allow-private-network can bypass this check

  @finding-SEC-013 @security @priority-high
  Scenario: Browser fetch blocks RFC1918 addresses
    When a user runs "hpx fetch http://10.0.0.1"
    Then the request SHALL fail with a forbidden address error

  @finding-SEC-013 @security @priority-high
  Scenario: Browser fetch blocks IPv6 loopback
    When a user runs "hpx fetch http://[::1]"
    Then the request SHALL fail with a forbidden address error

  @finding-SEC-013 @security @priority-high
  Scenario: Browser fetch blocks link-local addresses
    When a user runs "hpx fetch http://169.254.1.1"
    Then the request SHALL fail with a forbidden address error

  @finding-SEC-013 @security @priority-medium
  Scenario: Browser fetch allows private networks with explicit flag
    When a user runs "hpx fetch http://10.0.0.1 --allow-private-network"
    Then the request SHALL proceed normally
    And the SSRF check SHALL be bypassed

  @finding-SEC-013 @security @priority-medium
  Scenario: Browser fetch allows public IP addresses
    When a user runs "hpx fetch http://93.184.216.34"
    Then the request SHALL succeed without triggering the SSRF check

  @finding-SEC-013 @security @priority-medium
  Scenario: Browser scrape applies SSRF protection to all URLs
    When a user runs "hpx scrape http://192.168.1.1 http://example.com"
    Then the request to 192.168.1.1 SHALL fail
    And the request to example.com SHALL succeed

  @finding-SEC-013 @security @priority-low
  Scenario: Hostname-based SSRF is not blocked (only IP literals)
    When a user runs "hpx fetch http://localhost"
    Then the request SHALL proceed
    And a warning SHALL note that hostname-based SSRF requires DNS-level checks

  # --- TLS Key Log Gating ---

  @finding-SEC-014 @security @priority-high
  Scenario: SSLKEYLOGFILE is ignored in release builds without keylog feature
    Given the hpx crate is compiled in release mode without the "keylog" feature
    When the SSLKEYLOGFILE environment variable is set
    Then TLS session keys SHALL NOT be written to the key log file

  @finding-SEC-014 @security @priority-medium
  Scenario: SSLKEYLOGFILE works when keylog feature is enabled
    Given the hpx crate is compiled with the "keylog" feature
    When the SSLKEYLOGFILE environment variable is set to a valid path
    Then TLS session keys SHALL be written to the specified file

  @finding-SEC-014 @security @priority-medium
  Scenario: Key log feature is opt-in via Cargo feature flag
    Given a downstream user adds hpx to their Cargo.toml without "keylog"
    Then the KeyLog::from_env() call SHALL return a no-op KeyLog
    And no file I/O for key logging SHALL occur

  # --- Path Traversal Prevention in Downloads ---

  @finding-SEC-012 @security @priority-high
  Scenario: Download rejects paths containing parent directory traversal
    When a user runs "hpx dl add https://example.com/file.bin -o ../../etc/passwd"
    Then the download SHALL fail with a path traversal error
    And no file SHALL be written outside the working directory

  @finding-SEC-012 @security @priority-high
  Scenario: Download rejects absolute paths outside working directory
    When a user runs "hpx dl add https://example.com/file.bin -o /etc/passwd"
    Then the download SHALL fail with a path traversal error

  @finding-SEC-012 @security @priority-medium
  Scenario: Download accepts paths relative to the download root
    When a user runs "hpx dl add https://example.com/file.bin -o ./downloads/file.bin"
    Then the download SHALL proceed normally
    And the file SHALL be written to ./downloads/file.bin

  @finding-SEC-012 @security @priority-medium
  Scenario: Download accepts absolute paths when configured to allow
    Given download root is configured to "/tmp/hpx-downloads"
    When a user runs "hpx dl add https://example.com/file.bin -o /tmp/hpx-downloads/file.bin"
    Then the download SHALL proceed normally
