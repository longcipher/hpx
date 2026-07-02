Feature: SharedSession explicit dependency injection
  As a developer testing hpx-browser components
  I want SharedSession to be explicitly constructed and passed around
  So that tests can create isolated sessions without shared global state

  Background:
    Given the old `static SHARED_SESSION` OnceLock has been removed

  Scenario: Page creates its own SharedSession by default
    When I create a new Page without explicitly providing a session
    Then the Page owns a private SharedSession
    And cookies set in one Page are not visible in another Page

  Scenario: Two Pages can share a session when explicitly wired
    When I create an Arc<SharedSession> and pass it to two Page instances
    And I set a cookie in Page A
    Then Page B can see that cookie on its next request to the same domain

  Scenario: Cookie jar is isolated per-session
    When I create two independent Pages
    And I call `clear_cookies_for_domain` on one Page's session
    Then the other Page's cookies are unaffected
