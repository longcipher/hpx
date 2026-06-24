Feature: Stealth Profile
  As an hpx client user
  I want to use rich browser stealth profiles
  So that my HTTP requests match real browser fingerprints

  Scenario: Validate Chrome 148 Windows profile
    Given a Chrome 148 Windows stealth profile
    When I validate the profile
    Then validation should succeed with no errors

  Scenario: Validate Chrome 148 macOS profile
    Given a Chrome 148 macOS stealth profile
    When I validate the profile
    Then validation should succeed with no errors

  Scenario: Validate Firefox 135 macOS profile
    Given a Firefox 135 macOS stealth profile
    When I validate the profile
    Then validation should succeed with no errors

  Scenario: Random desktop profile selection
    When I call random_desktop() 10 times
    Then I should get at least 2 different profiles
    And all returned profiles should validate successfully

  Scenario: Profile to Emulation conversion
    Given a Chrome 148 macOS stealth profile
    When I convert it to an Emulation
    Then the emulation should have correct User-Agent header
    And the emulation should have correct Accept header

  Scenario: Profile to TlsOptions conversion
    Given a Chrome 148 macOS stealth profile
    When I convert it to TlsOptions
    Then the TLS options should have Chrome cipher list
    And the TLS options should have correct curves

  Scenario: Locale override
    Given a Chrome 148 Windows base profile
    When I call with_locale with "fr-FR", ["fr-FR", "fr"], "Europe/Paris"
    Then the profile language should be "fr-FR"
    And the profile timezone should be "Europe/Paris"
