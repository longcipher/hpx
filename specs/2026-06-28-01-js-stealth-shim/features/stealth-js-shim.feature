Feature: JS Stealth Shim
  As an hpx browser engine user
  I want JS-level anti-detection shimming
  So that bot managers cannot detect headless browsing via JavaScript API checks

  Background:
    Given a BrowserJsRuntime with stealth bootstrap loaded
    And a Chrome 148 Windows StealthProfile

  Scenario: Navigator userAgent matches profile
    When I set the user agent from the profile
    And I run page init
    Then evaluating "navigator.userAgent" should contain "Chrome/148"
    And evaluating "navigator.userAgent" should contain "Windows NT 10.0"

  Scenario: Navigator platform matches profile
    When I set the platform from the profile
    And I run page init
    Then evaluating "navigator.platform" should equal "Win32"

  Scenario: WebDriver is hidden
    When I run page init
    Then evaluating "navigator.webdriver" should equal "false"
    And evaluating "Object.getOwnPropertyDescriptor(navigator, 'webdriver')" should be undefined

  Scenario: WebDriver is on prototype chain
    When I run page init
    Then evaluating "Object.getOwnPropertyDescriptor(Navigator.prototype, 'webdriver')" should have a getter
    And evaluating "navigator instanceof Navigator" should be true

  Scenario: Chrome object exists with required properties
    When I run page init
    Then evaluating "typeof window.chrome" should equal "object"
    And evaluating "typeof window.chrome.app" should equal "object"
    And evaluating "typeof window.chrome.runtime" should equal "object"
    And evaluating "typeof window.chrome.csi" should equal "function"
    And evaluating "typeof window.chrome.loadTimes" should equal "function"

  Scenario: Chrome csi returns timing data
    When I run page init
    Then evaluating "typeof window.chrome.csi().onloadT" should equal "number"
    And evaluating "typeof window.chrome.csi().startE" should equal "number"

  Scenario: Chrome loadTimes returns timing data
    When I run page init
    Then evaluating "typeof window.chrome.loadTimes().requestTime" should equal "number"
    And evaluating "typeof window.chrome.loadTimes().finishLoadTime" should equal "number"

  Scenario: Screen dimensions from profile
    When I set the platform from the profile
    And I run page init
    Then evaluating "screen.width" should be a number greater than 0
    And evaluating "screen.height" should be a number greater than 0
    And evaluating "screen.colorDepth" should equal "24"

  Scenario: Navigator plugins match Chrome
    When I run page init
    Then evaluating "navigator.plugins.length" should equal "5"
    And evaluating "navigator.mimeTypes.length" should equal "2"

  Scenario: Navigator hardwareConcurrency is realistic
    When stealth mode is enabled
    And I run page init
    Then evaluating "navigator.hardwareConcurrency" should be a number between 4 and 16

  Scenario: Navigator deviceMemory is realistic
    When stealth mode is enabled
    And I run page init
    Then evaluating "navigator.deviceMemory" should be either 4 or 8

  Scenario: sec-ch-ua GREASE brands have correct format
    When I run page init
    Then evaluating "navigator.userAgentData.brands.length" should equal "3"
    And evaluating "navigator.userAgentData.brands[0].brand" should contain "Not"
    And evaluating "navigator.userAgentData.brands[1].brand" should equal "Chromium"
    And evaluating "navigator.userAgentData.brands[2].brand" should equal "Google Chrome"

  Scenario: sec-ch-ua GREASE version matches Chrome major
    When I run page init
    Then evaluating "navigator.userAgentData.brands[1].version" should equal "148"

  Scenario: userAgentData getHighEntropyValues returns consistent data
    When I run page init
    Then evaluating "navigator.userAgentData.getHighEntropyValues(['architecture']).then(v => v.architecture)" should resolve to a string
    And evaluating "navigator.userAgentData.getHighEntropyValues(['platform']).then(v => v.platform)" should resolve to "Windows"

  Scenario: Notification permission defaults to default
    When I run page init
    Then evaluating "Notification.permission" should equal "default"

  Scenario: WebGLRenderingContext stub exists
    When I run page init
    Then evaluating "typeof WebGLRenderingContext" should equal "function"
    And evaluating "typeof WebGL2RenderingContext" should equal "function"

  Scenario: Performance memory has realistic values
    When I run page init
    Then evaluating "typeof performance.memory.jsHeapSizeLimit" should equal "number"
    And evaluating "performance.memory.jsHeapSizeLimit" should be greater than 0

  Scenario: Canvas toDataURL returns consistent hash per session
    When I run page init
    Then evaluating "document.createElement('canvas').toDataURL().length" should be greater than 0
    And calling toDataURL twice should return the same value

  Scenario: Canvas toDataURL returns different hash across sessions
    When I run page init in session A
    And I run page init in session B
    Then canvas toDataURL in session A should differ from session B

  Scenario: AudioContext sample rate is realistic
    When I run page init
    Then evaluating "new AudioContext().sampleRate" should be either 44100 or 48000

  Scenario: Battery level is between 0 and 1
    When I run page init
    Then evaluating "navigator.getBattery().then(b => b.level).then(l => l >= 0 && l <= 1)" should resolve to "true"

  Scenario: Geolocation returns coordinates
    When I run page init
    Then evaluating "new Promise(r => navigator.geolocation.getCurrentPosition(p => r(p.coords.latitude)))" should resolve to a number

  Scenario: Navigator permissions query returns prompt for sensitive APIs
    When I run page init
    Then evaluating "navigator.permissions.query({name:'camera'}).then(p => p.state)" should equal "prompt"
    And evaluating "navigator.permissions.query({name:'microphone'}).then(p => p.state)" should equal "prompt"
