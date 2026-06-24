Feature: hpx fetch — Browser-based page fetching
  As a user
  I want to fetch and render web pages via the browser engine
  So that I can get content from JS-heavy pages without headless Chrome

  Scenario: Fetch page and dump HTML
    Given a test server serving a page with JavaScript
    When I run `hpx fetch --dump html <url>`
    Then the output contains the rendered HTML
    And the output includes JavaScript-generated content

  Scenario: Fetch page and dump text
    Given a test server serving a page with text content
    When I run `hpx fetch --dump text <url>`
    Then the output contains readable text
    And the output does not contain HTML tags

  Scenario: Fetch page and dump links
    Given a test server serving a page with links
    When I run `hpx fetch --dump links <url>`
    Then the output contains all links as tab-separated values
    And each line has a URL and optional text

  Scenario: Fetch page and dump markdown
    Given a test server serving an HTML page
    When I run `hpx fetch --dump markdown <url>`
    Then the output contains valid Markdown
    And headings, links, and paragraphs are converted

  Scenario: Fetch page with JavaScript evaluation
    Given a test server serving a page with JavaScript
    When I run `hpx fetch -e "document.title" <url>`
    Then the output contains the page title

  Scenario: Fetch page with selector wait
    Given a test server serving a page that loads content asynchronously
    When I run `hpx fetch --selector "#dynamic-content" <url>`
    Then the command waits for the selector to appear
    And the output contains the rendered HTML

  Scenario: Fetch raw bytes (dump original)
    Given a test server serving a binary file
    When I run `hpx fetch --dump original <url>`
    Then the output is the raw HTTP response body
    And the output is binary-safe (no trailing newline)

  Scenario: Fetch page and dump assets
    Given a test server serving a page with sub-resources
    When I run `hpx fetch --dump assets <url>`
    Then the output is NDJSON
    And each line contains a URL and resource type

  Scenario: Fetch page and dump cookies
    Given a test server setting cookies
    When I run `hpx fetch --dump cookies <url>`
    Then the output is a JSON array of cookies
    And HttpOnly cookies are included

  Scenario: Fetch with output file
    Given a test server serving a page
    When I run `hpx fetch --dump html -o /tmp/output.html <url>`
    Then the file /tmp/output.html contains the rendered HTML

  Scenario: Fetch with quiet mode
    Given a test server serving a page
    When I run `hpx fetch --quiet --dump html <url>`
    Then no progress messages are printed to stderr
    And the output contains the rendered HTML

  Scenario: Fetch with stealth mode
    Given a test server serving a page
    When I run `hpx fetch --stealth --dump html <url>`
    Then the request includes stealth headers
    And tracker domains are blocked

  Scenario: Fetch with timeout
    Given a test server that responds slowly
    When I run `hpx fetch --timeout 2 --dump html <url>`
    Then the command exits with a timeout error
    And the error message includes the timeout duration

  Scenario: Fetch with post-load settle
    Given a test server serving a page with delayed JavaScript
    When I run `hpx fetch --wait 3 --dump html <url>`
    Then the command waits for the settle period
    And JavaScript-generated content is included in output
