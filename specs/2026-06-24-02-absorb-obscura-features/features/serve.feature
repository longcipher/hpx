Feature: hpx serve — CDP WebSocket server
  As a user
  I want to run a local CDP server
  So that Puppeteer/Playwright can connect and automate the browser

  Scenario: Start CDP server on default port
    When I run `hpx serve`
    Then the server starts on port 9222
    And the banner shows the CDP WebSocket URL

  Scenario: Start CDP server on custom port
    When I run `hpx serve --port 9333`
    Then the server starts on port 9333
    And the banner shows ws://127.0.0.1:9333/devtools/browser

  Scenario: Start CDP server with stealth mode
    When I run `hpx serve --stealth`
    Then the server starts with stealth enabled
    And tracker blocking is active

  Scenario: Start multi-worker CDP server
    When I run `hpx serve --workers 4`
    Then 4 worker processes are spawned
    And a load balancer runs on the main port

  Scenario: CDP server accepts WebSocket connections
    Given the CDP server is running
    When a WebSocket client connects to /devtools/browser
    Then the connection is established
    And the client can send CDP commands

  Scenario: CDP server handles /json endpoint
    Given the CDP server is running
    When an HTTP client requests /json
    Then the response contains the list of targets

  Scenario: CDP server handles /json/version endpoint
    Given the CDP server is running
    When an HTTP client requests /json/version
    Then the response contains browser version info

  Scenario: CDP server with proxy
    When I run `hpx serve --proxy http://proxy:8080`
    Then the server starts with the proxy configured
    And browser requests go through the proxy

  Scenario: CDP server with allow-file-access
    When I run `hpx serve --allow-file-access`
    Then CDP clients can navigate to file:// URLs

  Scenario: CDP server with quiet mode
    When I run `hpx serve --quiet`
    Then no logs are printed
    And the server runs silently
