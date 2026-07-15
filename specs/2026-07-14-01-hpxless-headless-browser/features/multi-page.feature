Feature: Multiple concurrent pages
  As a developer running parallel scraping
  I want multiple pages to operate concurrently
  So that I can scrape many pages at once

  Scenario: 10 concurrent pages navigate simultaneously
    Given hpxless is started with port 0
    When I open 10 concurrent CDP connections
    And each connection navigates to "data:text/html,<p id='pid'>page</p>"
    And each connection evaluates "document.querySelector('p').textContent"
    Then all 10 connections receive the result "page"
