Feature: Emulation Browser Profiles
  As an HTTP client user
  I want to emulate specific browser versions
  So that my requests match real browser fingerprints

  Background:
    Given the hpx-emulation crate is loaded

  Scenario: All major browser families are represented
    Then the Emulation enum should have variants for Chrome, Firefox, Safari, Edge, Opera, and OkHttp

  Scenario: Chrome versions cover v100 through v149
    When I enumerate Chrome emulation variants
    Then versions 100 through 149 should be available

  Scenario: Firefox versions cover v109 through v151
    When I enumerate Firefox emulation variants
    Then versions 109 through 151 should be available

  Scenario: Safari versions cover v15.3 through v26.4
    When I enumerate Safari emulation variants
    Then versions 15 through 26 should be available including iOS and iPad variants

  Scenario: Opera versions cover v116 through v131
    When I enumerate Opera emulation variants
    Then versions 116 through 131 should be available

  Scenario: Edge versions cover v101 through v148
    When I enumerate Edge emulation variants
    Then versions 101 through 148 should be available

  Scenario: Total variant count meets threshold
    Then the total Emulation variant count should be at least 120

  Scenario: Each variant produces valid emulation
    When I create an emulation from any variant
    Then the emulation should have non-empty headers
    And the emulation should have a valid User-Agent header
