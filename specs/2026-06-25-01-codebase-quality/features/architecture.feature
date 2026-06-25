Feature: Architecture Improvements

  Consolidate error type hierarchies and deprecate config group structs
  to reduce API surface duplication.

  @direction-2 @priority-medium
  Scenario: Unified error type for hpx crate
    Given the hpx crate error handling
    When an error occurs in the core client
    Then it should be represented by a single hpx::Error type
    And the core::Error and client::Error types should be removed
    And duplicate BoxError and TimedOut aliases should be eliminated

  @direction-2 @priority-medium
  Scenario: Error conversion paths are preserved
    Given the unified hpx::Error type
    When a core::Error is produced internally
    Then it should convert to hpx::Error via From
    And the error kind variants should be preserved
    And downstream code matching on error kinds should still work

  @direction-3 @priority-medium
  Scenario: Config group structs are deprecated
    Given the config_groups module in hpx
    When a user configures the client
    Then ClientBuilder methods should be the primary API
    And config group structs should emit deprecation warnings
    And the config group code should delegate to ClientBuilder

  @direction-3 @priority-medium
  Scenario: ClientBuilder methods remain the single source of truth
    Given a new timeout option is added to ClientBuilder
    When the option is configured
    Then it should be available on ClientBuilder directly
    And it should NOT need to be duplicated in config groups
