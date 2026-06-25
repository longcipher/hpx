Feature: Tech Debt Reduction

  Remove dead code, dead feature flags, and unnecessary blanket lint
  disables across the workspace.

  @finding-DEBT-04 @priority-medium
  Scenario: hpx core crate lints catch dead code
    Given the hpx crate
    When clippy runs with pedantic + nursery lints
    Then dead code warnings should be reported
    And the blanket #![allow(dead_code)] should be removed

  @finding-DEBT-05 @priority-medium
  Scenario: hpx-browser has clippy correctness lints enabled
    Given the hpx-browser crate
    When clippy runs with default lints
    Then clippy::all should be enforced
    And correctness warnings should be reported

  @finding-DEBT-02 @priority-low
  Scenario: Dead feature flags removed from hpx-dl
    Given the hpx-dl Cargo.toml features section
    When the crate is compiled
    Then the storage feature should not exist (never gated)
    And the progress feature should not exist (never gated)

  @finding-DEBT-08 @priority-low
  Scenario: fast_random wrapper removed
    Given the hpx util module
    When the codebase is searched for fast_random usage
    Then zero references should exist
    And call sites should use rand::random::<u64>() directly
