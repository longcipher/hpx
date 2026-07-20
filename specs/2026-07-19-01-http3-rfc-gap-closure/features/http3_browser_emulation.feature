Feature: HTTP/3 Browser Emulation
  As a high-performance HTTP client with fingerprint emulation
  I want to match the QUIC transport parameters and h3 SETTINGS of real browsers
  So that I can evade fingerprinting and behave indistinguishably from a real browser

  Background:
    Given the `http3` Cargo feature is enabled
    And the `hpx-emulation` crate is available
    And `Http3Options` has fields for QUIC transport parameters, h3 SETTINGS, and Initial Packet padding

  # REQ-08, REQ-09
  # C-18

  Scenario: Chrome 96 http3_options matches real Chrome
    When I invoke `http3_options!(Chrome96)`
    Then the resulting `Http3Options` has:
      | field                           | value   |
      | `max_idle_timeout`              | `30s`   |
      | `stream_receive_window`         | `8 MiB` |
      | `max_concurrent_bidi_streams`   | `100`   |
      | `qpack_max_table_capacity`      | `4096`  |
      | `qpack_blocked_streams`         | `100`   |
      | `initial_packet_padding`        | `1200`  |

  Scenario: Chrome 143 http3_options matches real Chrome
    When I invoke `http3_options!(Chrome143)`
    Then the resulting `Http3Options` equals `Http3Options::default()`
    And the `initial_packet_padding` is `1200` bytes

  Scenario: Firefox 88 http3_options matches real Firefox
    When I invoke `http3_options!(Firefox88)`
    Then the resulting `Http3Options` has:
      | field                           | value   |
      | `max_idle_timeout`              | `30s`   |
      | `stream_receive_window`         | `1 MiB` |
      | `max_concurrent_bidi_streams`   | `100`   |
      | `qpack_max_table_capacity`      | `0`     |
      | `initial_packet_padding`        | `1232`  |

  Scenario: Safari 14 http3_options matches real Safari
    When I invoke `http3_options!(Safari14)`
    Then the resulting `Http3Options` has:
      | field                           | value   |
      | `max_idle_timeout`              | `30s`   |
      | `stream_receive_window`         | `8 MiB` |
      | `max_concurrent_bidi_streams`   | `100`   |
      | `qpack_max_table_capacity`      | `4096`  |

  Scenario: Edge 96 http3_options matches real Edge
    When I invoke `http3_options!(Edge96)`
    Then the resulting `Http3Options` equals `http3_options!(Chrome96)`
    And the User-Agent string is Edge-specific (`Edg/96`)

  Scenario: emulation() applies http3_options automatically
    When I build a `Client` with `ClientBuilder::new().emulation(Chrome143).build()`
    Then the client's `http3_options` matches `http3_options!(Chrome143)`
    And no explicit `http3_options()` call was needed

  Scenario: QUIC transport parameters match browser fingerprint
    When I invoke `http3_options!(Chrome143)` and `http3_options!(Firefox88)`
    Then the two `Http3Options` produce distinct `quinn::TransportConfig` values
    And each `TransportConfig` matches the corresponding browser's known transport parameters

  Scenario: QUIC Initial Packet padding matches browser
    When I invoke `http3_options!(Chrome143)` and start a QUIC connection
    Then the first Initial Packet is padded to `1200` bytes
    And a fingerprinter observing the wire sees Chrome-like padding

  Scenario: CLI --http3 forces h3
    When I run `hpx --http3 https://127.0.0.1:4433/`
    Then the request is sent over HTTP/3
    And the output shows `Version: HTTP/3`

  Scenario: CLI --emulation Chrome143 applies http3_options
    When I run `hpx --http3 --emulation Chrome143 https://127.0.0.1:4433/`
    Then the QUIC transport parameters match `http3_options!(Chrome143)`
    And the Initial Packet padding matches Chrome 143
