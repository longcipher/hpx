@finding-HOT-01 @finding-HOT-02 @category-performance @priority-high
Feature: Hotpath profiling across hpx and hpx-dl crates
  As a developer optimizing the hpx workspace
  I want hotpath instrumentation on all critical hot paths
  So that I can profile and identify performance bottlenecks in production

  @finding-HOT-01
  Scenario: hpx crate exposes hotpath feature flag
    Given the hpx crate Cargo.toml
    When I inspect the [features] section
    Then there is a "hotpath" feature that depends on "dep:hotpath"
    And hotpath is listed as an optional dependency in [dependencies]

  @finding-HOT-01
  Scenario: hpx connection pool acquire is instrumented
    Given the hpx client connection pool code
    When a connection is acquired from the pool
    Then the acquire function has #[cfg_attr(feature = "hotpath", hotpath::measure)]
    And profiling data is captured when hotpath feature is enabled

  @finding-HOT-01
  Scenario: hpx request building is instrumented
    Given the hpx RequestBuilder
    When a request is built and sent
    Then the send function has #[cfg_attr(feature = "hotpath", hotpath::measure)]
    And profiling data is captured when hotpath feature is enabled

  @finding-HOT-02
  Scenario: hpx-dl crate exposes hotpath feature flag
    Given the hpx-dl crate Cargo.toml
    When I inspect the [features] section
    Then there is a "hotpath" feature that depends on "dep:hotpath"
    And hotpath is listed as an optional dependency in [dependencies]

  @finding-HOT-02
  Scenario: hpx-dl segment download is instrumented
    Given the hpx-dl segment download code
    When a segment is downloaded
    Then download_segment_with_options has #[cfg_attr(feature = "hotpath", hotpath::measure)]
    And profiling data is captured when hotpath feature is enabled

  @finding-HOT-02
  Scenario: hpx-dl speed limiter is instrumented
    Given the hpx-dl SpeedLimiter
    When wait_for is called
    Then refill_tokens and try_consume have #[cfg_attr(feature = "hotpath", hotpath::measure)]
    And profiling data is captured when hotpath feature is enabled

  @finding-HOT-02
  Scenario: hpx-dl segment state building is instrumented
    Given the hpx-dl engine segment state code
    When build_segment_states is called
    Then the function has #[cfg_attr(feature = "hotpath", hotpath::measure)]

  @finding-HOT-02
  Scenario: hpx-dl checksum verification is instrumented
    Given the hpx-dl checksum code
    When compute_checksum is called
    Then the function has #[cfg_attr(feature = "hotpath", hotpath::measure)]

  @finding-PERF-15 @category-performance @priority-medium
  Scenario: ProxyPool select avoids deep clone of Matcher
    Given the ProxyPool with multiple proxies
    When select() is called on every request
    Then the Matcher is wrapped in Arc for O(1) clone
    And request latency does not increase with Matcher complexity

  @finding-PERF-13 @category-performance @priority-low
  Scenario: resume_state_from_segments iterates segments once
    Given a list of segment states
    When resume_state_from_segments is called
    Then completed_segments and bytes_completed are computed in a single pass
    And the function iterates the segments slice exactly once

  @finding-PERF-14 @category-performance @priority-low
  Scenario: Segment state building uses ahash HashSet
    Given the build_segment_states and filter_remaining_segments functions
    When completed indices are checked
    Then ahash::HashSet is used instead of std::collections::HashSet
    And the project's pre-seeded HASHER from hash.rs is used

  @finding-PERF-16 @category-performance @priority-low
  Scenario: Checksum buffer is sized for large file throughput
    Given the checksum computation code
    When computing a checksum for a large file
    Then the read buffer is at least 64 KiB
    And syscall count is reduced compared to 8 KiB buffer
