Feature: hpx-cli WebSocket Reconnection
  As a user maintaining a WebSocket connection
  I want automatic reconnection when the connection drops
  So I don't lose my session

  Scenario: WebSocket with reconnection enabled
    Given a WebSocket server is available
    When I run "hpx ws://localhost:8080 --reconnect"
    And the connection drops
    Then the client attempts to reconnect
    And exponential backoff is applied
    And the connection is re-established

  Scenario: WebSocket without reconnection
    Given a WebSocket server is available
    When I run "hpx ws://localhost:8080"
    And the connection drops
    Then the client exits immediately
