Feature: OpenSSL TLS Backend Support
  As a developer using hpx in an OpenSSL-based infrastructure
  I want to use OpenSSL as my TLS backend
  So that I can integrate with existing OpenSSL security policies and compliance requirements

  @openssl @tls @compilation
  Scenario: Compile hpx with openssl-tls feature enabled
    Given the hpx crate is configured with the "openssl-tls" feature
    And the default "boring" feature is disabled
    When the crate is compiled
    Then the compilation succeeds without errors
    And the OpenSSL TLS backend is available for use

  @openssl @tls @vendored
  Scenario: Compile hpx with vendored OpenSSL
    Given the hpx crate is configured with the "openssl-vendored" feature
    And the default "boring" feature is disabled
    When the crate is compiled
    Then the compilation succeeds with a statically linked OpenSSL library
    And the OpenSSL TLS backend is available for use

  @openssl @tls @connectivity
  Scenario: Establish HTTPS connection using OpenSSL backend
    Given hpx is configured with the "openssl-tls" backend
    And a valid HTTPS server is available
    When an HTTPS GET request is sent
    Then the TLS handshake completes successfully using OpenSSL
    And the response body is received correctly

  @openssl @tls @mtls
  Scenario: Mutual TLS authentication with OpenSSL backend
    Given hpx is configured with the "openssl-tls" backend
    And a client identity is loaded from a PEM bundle
    And a custom certificate store includes the server's CA
    When an HTTPS request is sent to an mTLS-enabled server
    Then the client certificate is presented during the TLS handshake
    And the server accepts the connection and returns a successful response

  @openssl @tls @identity
  Scenario: Load client identity from PEM format
    Given hpx is configured with the "openssl-tls" backend
    When an Identity is created from a combined certificate and key PEM
    Then the identity is successfully parsed
    And it can be used for client certificate authentication

  @openssl @tls @identity
  Scenario: Load client identity from PKCS12 DER format
    Given hpx is configured with the "openssl-tls" backend
    When an Identity is created from a PKCS12 DER archive with a password
    Then the identity is successfully parsed
    And it can be used for client certificate authentication

  @openssl @tls @session-cache
  Scenario: TLS session resumption via session cache
    Given hpx is configured with the "openssl-tls" backend
    And PSK (pre-shared key) session resumption is enabled
    When two HTTPS requests are made to the same server
    Then the second request reuses the TLS session from the first request
    And the session cache correctly stores and retrieves sessions

  @openssl @tls @degradation
  Scenario: BoringSSL-specific features are silently ignored under OpenSSL
    Given hpx is configured with the "openssl-tls" backend
    When TLS options include GREASE, ECH, ALPS, or extension permutation settings
    Then the TLS connector builds successfully without errors
    And the unsupported options are silently treated as no-ops
    And a debug-level log message is emitted for significant omissions

  @openssl @tls @precedence
  Scenario: BoringSSL takes precedence when both features are enabled
    Given the hpx crate is configured with both "boring" and "openssl-tls" features
    When the crate is compiled
    Then the BoringSSL backend is used
    And the OpenSSL backend code is not compiled

  @openssl @tls @compatibility
  Scenario: Existing backends remain unaffected
    Given the hpx crate has the "openssl-tls" feature added
    When the crate is compiled with the default "boring" feature
    Then the BoringSSL backend works identically to before the change
    And no existing functionality is broken
