Feature: hpx mcp — Model Context Protocol server
  As a user
  I want to run an MCP server for AI agent integration
  So that AI agents can automate browser tasks

  Scenario: MCP server starts in stdio mode
    When I run `hpx mcp`
    Then the server reads JSON-RPC from stdin
    And the server writes JSON-RPC to stdout

  Scenario: MCP server responds to initialize
    Given the MCP server is running in stdio mode
    When I send an initialize request
    Then the response contains server info and capabilities

  Scenario: MCP server responds to tools/list
    Given the MCP server is running in stdio mode
    When I send a tools/list request
    Then the response contains 32+ tools
    And each tool has name, description, and inputSchema

  Scenario: MCP server handles browser_navigate tool
    Given the MCP server is running in stdio mode
    When I call browser_navigate with a URL
    Then the browser navigates to the URL
    And the response contains the page title

  Scenario: MCP server handles browser_snapshot tool
    Given the MCP server has navigated to a page
    When I call browser_snapshot
    Then the response contains the page URL, title, and text

  Scenario: MCP server handles browser_click tool
    Given the MCP server has navigated to a page with a button
    When I call browser_click with the button selector
    Then the button is clicked
    And the page state changes

  Scenario: MCP server handles browser_fill tool
    Given the MCP server has navigated to a page with a form
    When I call browser_fill with a selector and value
    Then the input value is set
    And input/change events are triggered

  Scenario: MCP server handles browser_evaluate tool
    Given the MCP server has navigated to a page
    When I call browser_evaluate with a JavaScript expression
    Then the expression is evaluated
    And the result is returned

  Scenario: MCP server handles browser_markdown tool
    Given the MCP server has navigated to a page
    When I call browser_markdown
    Then the response contains the page as Markdown

  Scenario: MCP server handles browser_links tool
    Given the MCP server has navigated to a page with links
    When I call browser_links
    Then the response contains all links as JSON

  Scenario: MCP server starts in HTTP mode
    When I run `hpx mcp --http --port 3000`
    Then the server listens on port 3000
    And accepts POST requests to /mcp

  Scenario: MCP HTTP server handles CORS
    Given the MCP server is running in HTTP mode
    When I send an OPTIONS request with Origin header
    Then the response contains CORS headers
