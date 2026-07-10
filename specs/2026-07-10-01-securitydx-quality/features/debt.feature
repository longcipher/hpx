Feature: Tech debt reduction — Remove deprecated configuration types and dead code

  Background:
    Given a working hpx workspace with all crates compiled

  # --- Deprecated Config Groups ---

  @finding-DEBT-14 @debt @priority-high
  Scenario: Deprecated config groups are removed from the public API
    Given the hpx crate version is bumped to a breaking version
    When a user compiles code using TransportConfigOptions, PoolConfigOptions, TlsConfigOptions, ProtocolConfigOptions, or ProxyConfigOptions
    Then the compiler SHALL report that these types do not exist
    And the user SHALL be directed to use ClientBuilder methods directly

  @finding-DEBT-14 @debt @priority-medium
  Scenario: Internal code uses ClientBuilder methods directly
    Given the deprecated config_groups module is removed
    When the workspace is compiled
    Then no compilation errors SHALL occur from missing config group types
    And all existing tests SHALL pass

  @finding-DEBT-14 @debt @priority-medium
  Scenario: No allow(deprecated) annotations remain in hpx
    Given the deprecated config groups are fully removed
    When clippy runs on the hpx crate
    Then no allow(deprecated) annotations SHALL exist in builder.rs or lib.rs
