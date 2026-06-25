Feature: Test Coverage and CI Fixes

  Fix CI configuration to run all tests, and add missing unit tests
  for critical untested modules.

  @finding-TEST-02 @priority-high
  Scenario: CI runs all test suites
    Given the Justfile ci target
    When CI is executed
    Then lint should pass
    And unit tests should pass
    And integration tests should pass
    And BDD scenarios should pass

  @finding-TEST-03 @priority-medium
  Scenario: Developer test command runs workspace-wide tests
    Given the Justfile test target
    When a developer runs just test
    Then hpx-dl tests should be included
    And hpx integration tests should be included
    And hpx-streams, hpx-emulation, hpx-yawc tests should be included

  @finding-TEST-01 @priority-medium
  Scenario: hpx-streams codecs have unit tests
    Given the hpx-streams crate
    When cargo test is run for hpx-streams
    Then JSON array codec tests should pass
    And JSONL codec tests should pass
    And protobuf length-prefix codec tests should pass
    And arrow IPC codec tests should pass
    And CSV stream tests should pass

  @finding-TEST-04 @priority-medium
  Scenario: yawc codec has encode/decode roundtrip tests
    Given the yawc codec module
    When codec unit tests are run
    Then text frame roundtrip should pass
    And binary frame roundtrip should pass
    And close frame roundtrip should pass
    And ping/pong frame roundtrip should pass
    And all three payload length variants should be tested

  @finding-DX-04 @priority-low
  Scenario: Lint target does not include doc build
    Given the Justfile lint target
    When a developer runs just lint
    Then markdown, TOML, and Rust formatting checks should run
    And clippy should run
    And cargo machete should run
    And doc build should NOT be part of lint
