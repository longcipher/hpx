Feature: HTTP/3 Transport (QUIC + h3 framing)
  As a high-performance HTTP client
  I want to send requests over HTTP/3 (RFC 9114)
  So that I avoid head-of-line blocking and benefit from QUIC's stream multiplexing

  Background:
    Given a local h3 test server is running on `127.0.0.1:4433`
    And the `http3` Cargo feature is enabled
    And a `ClientBuilder` configured with `http3_only()` and a rustls client config trusting the test server's certificate

  # REQ-01, REQ-02, REQ-04, REQ-06, REQ-07, REQ-10, REQ-18, REQ-19
  # C-01, C-03, C-04, C-05, C-06, C-07, C-08, C-21, C-23

  Scenario: Ver::Http3 routes to h3 pool shard
    Given a `Client` built with `http3_only()`
    When I send a GET request to `https://127.0.0.1:4433/`
    Then the request is routed via the `Ver::Http3` pool shard
    And no TCP connection is opened to the server

  Scenario: HTTP/3 ALPN h3 negotiated over QUIC
    When I send a GET request to `https://127.0.0.1:4433/`
    Then the QUIC handshake negotiates ALPN `h3`
    And the negotiated ALPN is exactly `b"h3"` (no draft versions)

  Scenario: Successful HTTP/3 GET request
    When I send a GET request to `https://127.0.0.1:4433/hello`
    Then the response status is `200 OK`
    And the response version is `Version::HTTP_3`
    And the response body is `hello, h3`

  Scenario: HTTP/3 POST with body and content-length
    When I send a POST request to `https://127.0.0.1:4433/echo` with body `ping` and `Content-Length: 4`
    Then the response status is `200 OK`
    And the response body is `ping`
    And the request was sent with `Content-Length: 4`

  Scenario: HTTP/3 streaming request body
    When I send a POST request to `https://127.0.0.1:4433/echo` with a streaming body yielding `["foo", "bar", "baz"]`
    Then the response status is `200 OK`
    And the response body is `foobarbaz`
    And no `Content-Length` header was sent on the request

  Scenario: HTTP/3 concurrent requests over single QUIC connection
    When I send 10 concurrent GET requests to `https://127.0.0.1:4433/concurrent`
    Then all 10 responses return `200 OK`
    And the server-side `ConnectionStats` reports exactly 1 QUIC connection was used

  Scenario: HTTP/3 connection failure surfaces typed error
    Given the client is pointed at `https://127.0.0.1:1/` (unused port)
    When I send a GET request
    Then the request fails with an `Error` where `is_connect()` is `true`
    And the inner error is `H3Error::Handshake { source: quinn::ConnectionError::TimedOut }`

  Scenario: HTTP/3 reconnection after server closes
    Given a successful GET request has been made
    When the server closes the QUIC connection
    And I send another GET request
    Then the first attempt fails with `H3Error::IdleClose` or `H3Error::StreamReset`
    And a subsequent attempt succeeds with `200 OK` after pool reconnection

  Scenario: HTTP/3 STOP_SENDING with H3_NO_ERROR is graceful
    When the server sends a `STOP_SENDING` frame with error code `H3_NO_ERROR` (0x0100)
    Then the response stream ends gracefully (empty body, no error surfaced)

  Scenario: HTTP/3 STOP_SENDING with H3_INTERNAL_ERROR surfaces error
    When the server sends a `STOP_SENDING` frame with error code `H3_INTERNAL_ERROR` (0x0102)
    Then the response stream fails with `H3Error::StreamReset`
    And the resulting `Error` satisfies `is_body()`

  Scenario: HTTP/3 request body mid-stream error surfaces is_body error
    When I send a POST request with a streaming body that errors after the first chunk
    Then the request fails with an `Error` where `is_request()` is `true`
    And the inner error satisfies `is_body()`

  Scenario: HTTP/2 path is unaffected by http3 feature flag
    Given the `http3` feature is enabled
    And a `Client` built WITHOUT `http3_only()` (default `HttpVersionPref::All`)
    When I send a GET request to an h2-only server
    Then the response version is `Version::HTTP_2`
    And the request used the existing h2 path unchanged

  Scenario: HTTP/3 0-RTT resumption on second connection
    Given a first GET request has completed with a full TLS handshake
    And `Http3Options.enable_0rtt` is `true`
    When the pool is drained and a second GET request is sent to the same authority
    Then the second connection uses 0-RTT resumption
    And `quinn::ConnectionStats` reports 0-RTT was accepted

  Scenario: HTTP/3 idle connection is closed after timeout
    Given `Http3Options.max_idle_timeout` is `5 seconds`
    And a GET request has completed
    When the connection is idle for `6 seconds`
    Then the QUIC connection is closed
    And a `GOAWAY` frame was sent before close
