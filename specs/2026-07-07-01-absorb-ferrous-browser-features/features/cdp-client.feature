Feature: CDP Client Connection
  As a user of hpx-browser
  I want to connect to Chrome via CDP
  So that I can drive real browser instances

  Background:
    Given a running Chrome instance with CDP enabled

  Scenario: Connect CdpClient to Chrome
    When I create a CdpClient connected to the Chrome WebSocket
    Then the connection should be established
    And the client should be ready to send commands

  Scenario: Send CDP command and receive response
    Given a connected CdpClient
    When I send a "Browser.getVersion" command
    Then I should receive a response with protocol version
    And the response should contain the browser product name

  Scenario: Register handler before send (race-free)
    Given a connected CdpClient
    When I register a response handler and immediately send a command
    Then the handler should receive the response
    And no race condition should occur

  Scenario: Broadcast events to multiple subscribers
    Given a connected CdpClient
    When I subscribe two listeners to events
    And a CDP event is received
    Then both listeners should receive the event

  Scenario: Fail all pending on disconnect
    Given a connected CdpClient with pending commands
    When the WebSocket connection is closed
    Then all pending commands should fail immediately
    And the error should indicate connection lost
