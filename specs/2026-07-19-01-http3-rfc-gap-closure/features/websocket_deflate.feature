Feature: WebSocket permessage-deflate (RFC 7692)
  As a high-performance HTTP client
  I want to compress WebSocket messages with permessage-deflate (RFC 7692)
  So that I reduce bandwidth for text-heavy WebSocket traffic

  Background:
    Given a local WebSocket test server is running on `ws://127.0.0.1:9001`
    And the `fastwebsockets` crate supports the `permessage-deflate` extension
    And the client offers `Sec-WebSocket-Extensions: permessage-deflate; client_max_window_bits`

  # REQ-12
  # C-16, C-17

  Scenario: permessage-deflate negotiated
    When I open a WebSocket connection offering `permessage-deflate`
    And the server accepts with `permessage-deflate; client_max_window_bits=10; server_max_window_bits=10`
    Then the negotiated extension is active on both sides
    And `client_max_window_bits` is `10`
    And `server_max_window_bits` is `10`

  Scenario: Compressed message round-trips
    Given an open WebSocket connection with `permessage-deflate` active
    When I send a text message containing 10 KiB of repeated `hello, world!`
    Then the server echoes back the message
    And the decompressed message matches the original 10 KiB
    And the on-wire frame size is less than 1 KiB (compressed)

  Scenario: Context takeover disabled resets state
    Given an open WebSocket connection with `client_no_context_takeover` negotiated
    When I send two consecutive text messages of 10 KiB each
    Then each message is compressed independently (no context carried over)
    And both messages decompress correctly on the server side

  Scenario: Window bits bounded
    When I offer `permessage-deflate; client_max_window_bits=15` and the server responds with `client_max_window_bits=8`
    Then the negotiated `client_max_window_bits` is `8`
    And the compression uses a window of at most `8` bits
    And the client does not crash or panic with the reduced window
