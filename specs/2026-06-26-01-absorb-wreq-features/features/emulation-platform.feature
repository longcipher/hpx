Feature: Platform-Aware Emulation
  As an HTTP client user
  I want platform-specific browser emulation
  So that my requests match the target OS fingerprint

  Scenario Outline: Platform produces correct User-Agent suffix
    When I create an emulation for Chrome147 on <platform>
    Then the User-Agent header should contain "<os_identifier>"

    Examples:
      | platform | os_identifier |
      | Windows  | Windows NT    |
      | MacOS    | Macintosh     |
      | Linux    | Linux         |
      | Android  | Android       |
      | iOS      | iPhone        |

  Scenario Outline: Platform produces correct sec-ch-ua-platform
    When I create an emulation for Chrome147 on <platform>
    Then the sec-ch-ua-platform header should be "<value>"

    Examples:
      | platform | value     |
      | Windows  | "Windows" |
      | MacOS    | "macOS"   |
      | Linux    | "Linux"   |
      | Android  | "Android" |
      | iOS      | "iOS"     |

  Scenario: Mobile platforms are correctly identified
    Then Platform::Android.is_mobile() should be true
    And Platform::IOS.is_mobile() should be true
    And Platform::Windows.is_mobile() should be false
    And Platform::MacOS.is_mobile() should be false
    And Platform::Linux.is_mobile() should be false

  Scenario: Default platform is Windows
    Then Platform::default() should be Windows

  Scenario: Weighted random produces realistic distribution
    When I select 10000 random emulations using weighted_random
    Then Chrome profiles should be selected approximately 71% of the time
    And Safari profiles should be selected approximately 15% of the time
    And Edge profiles should be selected approximately 5% of the time
