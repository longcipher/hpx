Feature: hpx scrape — Parallel multi-URL scraping
  As a user
  I want to scrape multiple URLs in parallel
  So that I can collect data from many pages efficiently

  Scenario: Scrape multiple URLs
    Given a test server serving multiple pages
    When I run `hpx scrape <url1> <url2> <url3>`
    Then the output is a JSON object with results
    And each result contains url, title, and time_ms

  Scenario: Scrape with concurrency limit
    Given a test server serving multiple pages
    When I run `hpx scrape --concurrency 2 <url1> <url2> <url3> <url4>`
    Then at most 2 workers run simultaneously
    And all URLs are scraped successfully

  Scenario: Scrape with JavaScript evaluation
    Given a test server serving pages with JavaScript
    When I run `hpx scrape -e "document.title" <url1> <url2>`
    Then each result contains the eval field with the title

  Scenario: Scrape with text format
    Given a test server serving multiple pages
    When I run `hpx scrape --format text <url1> <url2>`
    Then the output is tab-separated text
    And each line has time, URL, and title

  Scenario: Scrape with timeout
    Given a test server that responds slowly
    When I run `hpx scrape --timeout 2 <url>`
    Then the worker times out
    And the result contains an error message

  Scenario: Scrape with quiet mode
    Given a test server serving multiple pages
    When I run `hpx scrape --quiet <url1> <url2>`
    Then no progress messages are printed to stderr
    And the results are printed to stdout

  Scenario: Scrape with no URLs
    When I run `hpx scrape`
    Then the command exits with an error
    And the error message says "No URLs provided"
