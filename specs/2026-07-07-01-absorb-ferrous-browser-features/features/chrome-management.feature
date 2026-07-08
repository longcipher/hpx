Feature: Chrome Process Management
  As a user of hpx-browser
  I want to launch and manage Chrome processes
  So that I can use real browsers without manual setup

  Scenario: Detect Chrome binary on macOS
    Given the system has Chrome installed
    When I call find_chrome
    Then it should return the path to the Chrome binary

  Scenario: Find a free port
    When I call free_port
    Then it should return a valid TCP port number
    And the port should be available for binding

  Scenario: Launch Chrome with default config
    Given the default BrowserConfig
    When I call Browser::launch
    Then a Chrome process should be started
    And a CDP connection should be established
    And the Browser handle should be valid

  Scenario: Launch Chrome with custom viewport
    Given a BrowserConfig with viewport 1920x1080
    When I call Browser::launch
    Then Chrome should start with the specified viewport

  Scenario: Create new page via CDP
    Given a running Browser
    When I call Browser::new_page
    Then a new CdpPage should be returned
    And the page should have a valid session_id
    And the page should have a valid target_id

  Scenario: Create multiple pages
    Given a running Browser
    When I create 3 pages
    Then each page should have a unique session_id
    And events from one page should not affect others

  Scenario: Kill Chrome on Drop
    Given a running Browser
    When the Browser is dropped
    Then the Chrome process should be killed
    And the port should be freed
