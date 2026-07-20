Feature: WebSocket over HTTP/3 (RFC 9220)
  As a high-performance HTTP client
  I want to establish WebSocket connections over HTTP/3 using Extended CONNECT (RFC 9220)
  So that I benefit from QUIC's stream multiplexing and 0-RTT for WebSocket traffic

  Background:
    Given a local h3 test server supporting Extended CONNECT is running on `127.0.0.1:4433`
    And the `http3` Cargo feature is enabled
    And `Http3Options.enable_connect_protocol` is `true`
    And a `Client` built with `http3_only()` and `yawc` WebSocket support

  # REQ-14, REQ-19
  # C-15

  Scenario: WebSocket over h3 via Extended CONNECT
    When I open a WebSocket connection to `wss://127.0.0.1:4433/ws`
    Then the client sends an Extended CONNECT request with `:protocol = websocket`
    And the server responds with `200 OK` and `Sec-WebSocket-Accept` set
    And the WebSocket handshake completes successfully

  Scenario: WebSocket over h3 text message
    Given an open WebSocket connection to `wss://127.0.0.1:4433/echo`
    When I send a text message `hello, h3 ws`
    Then the server echoes back the text message `hello, h3 ws`
    And the message is received as a `Text` frame

  Scenario: WebSocket over h3 binary message
    Given an open WebSocket connection to `wss://127.0.0.1:4433/echo`
    When I send a binary message `b"\x00\x01\x02\x03"`
    Then the server echoes back the binary message `b"\x00\x01\x02\x03"`
    And the message is received as a `Binary` frame

  Scenario: WebSocket over h3 close handshake
    Given an open WebSocket connection to `wss://127.0.0.1:4433/echo`
    When I send a `Close` frame with code `1000` and reason `normal closure`
    Then the server responds with a `Close` frame with code `1000`
    And the h3 stream is closed cleanly
    And no error is surfaced to the application

  Scenario: WebSocket over h3 ping/pong
    Given an open WebSocket connection to `wss://127.0.0.1:4433/echo`
    When I send a `Ping` frame with payload `b"ping"`
    Then the server responds with a `Pong` frame with payload `b"ping"`
    And the application does not receive the Ping or Pong as a message
