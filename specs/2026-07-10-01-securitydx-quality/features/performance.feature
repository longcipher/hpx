Feature: Performance — Eliminate redundant allocations in hot paths

  Background:
    Given a working hpx workspace with all crates compiled

  # --- Checksum Buffer Allocation ---

  @finding-PERF-17 @performance @priority-high
  Scenario: Checksum computation reuses a pre-allocated buffer instead of allocating per call
    When compute_checksum is called for a file
    Then a 64 KiB buffer SHALL be reused across checksum operations
    And no heap allocation SHALL occur for the read buffer on each call

  @finding-PERF-17 @performance @priority-medium
  Scenario: Checksum computation produces the same hash with thread-local buffer
    When compute_checksum is called with the same file and algorithm
    Then the resulting hash SHALL be identical to the previous vec-based implementation
    And all existing checksum tests SHALL pass unchanged
