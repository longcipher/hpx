Feature: Stealth CLI Wiring
  As an hpx CLI user
  I want the --stealth flag to enable JS-level anti-detection
  So that CDP-connected browsers have consistent stealth across network and JS layers

  Background:
    Given hpx-cli is available

  Scenario: --stealth flag is accepted on serve subcommand
    When I run "hpx serve --help"
    Then the output should contain "--stealth"

  Scenario: --stealth flag defaults to false
    When I parse the serve command without --stealth
    Then stealth should be false

  Scenario: --stealth flag sets stealth to true
    When I parse the serve command with --stealth
    Then stealth should be true

  Scenario: CDP server receives stealth flag
    Given a CDP server started with stealth=true
    When I connect via CDP and evaluate "navigator.webdriver"
    Then the result should be "false"

  Scenario: CDP server without stealth has default navigator
    Given a CDP server started with stealth=false
    When I connect via CDP and evaluate "typeof navigator.webdriver"
    Then the result should indicate webdriver property exists

  Scenario: Stealth globals are injected when stealth=true
    Given a CDP server started with stealth=true
    When I connect via CDP and evaluate "globalThis.__hpx_stealth"
    Then the result should be "true"

  Scenario: Stealth globals are not injected when stealth=false
    Given a CDP server started with stealth=false
    When I connect via CDP and evaluate "typeof globalThis.__hpx_stealth"
    Then the result should be "undefined"
