Feature: Performance optimizations for hpx workspace
  As a developer using hpx for HFT workloads
  I want the hot paths to be as efficient as possible
  So that latency and throughput are not degraded by avoidable overhead

  # Finding 1: Redundant file seek per write chunk
  @finding-1 @perf @S
  Scenario: Segment download avoids redundant seeks
    Given a segment download is in progress
    When chunks are written to the file
    Then the file is sought only once before the write loop begins
    And subsequent chunks are written without additional seek calls

  # Finding 2: TLS connector rebuilt per WSS connection
  @finding-2 @perf @S
  Scenario: TLS connector is cached across WSS connections
    Given multiple WSS connections are established
    When each connection needs a TLS connector
    Then the root cert store and ClientConfig are constructed only once
    And subsequent connections reuse the cached TlsConnector

  # Finding 3: O(n×m) linear scan in segment filtering
  @finding-3 @perf @S
  Scenario: Segment filtering uses constant-time lookup
    Given a download with 50 segments where 40 are completed
    When filter_remaining_segments is called
    Then completed indices are checked via HashSet lookup in O(1)
    And the overall filtering runs in O(n) instead of O(n×m)

  @finding-3 @perf @S
  Scenario: build_segment_states uses constant-time lookup
    Given a download with many segments and completed indices
    When build_segment_states is called to reconstruct state
    Then completed_indices lookup uses a HashSet
    And state reconstruction runs in O(n)

  # Finding 4: CSV ReaderBuilder constructed per row
  @finding-4 @perf @S
  Scenario: CSV stream reuses ReaderBuilder across rows
    Given a CSV stream with thousands of rows
    When each row is deserialized
    Then the csv::ReaderBuilder is constructed once before the stream
    And the same builder configuration is reused for every row

  # Finding 5: DownloadEntry clone cascade
  @finding-5 @perf @M
  Scenario: State transitions avoid unnecessary full-entry clones
    Given a download entry with segments
    When update_entry is called for a state transition
    Then the modified entry is returned directly from the atomic update
    And no redundant clone occurs for persistence
    And to_record only clones fields needed for the persistence record

  # Finding 6: std::sync::mpsc replaced with crossbeam-channel
  @finding-6 @perf @S
  Scenario: Persistence channel uses crossbeam-channel
    Given the persistence handle is initialized
    When commands are sent to the persistence worker
    Then crossbeam_channel is used instead of std::sync::mpsc
    And the drop implementation properly drains pending commands

  # Finding 7: mark_segment_completed linear scan
  @finding-7 @perf @S
  Scenario: Segment completion uses indexed lookup
    Given a download entry with many segments
    When mark_segment_completed is called for a specific index
    Then the segment is found by direct index lookup
    And no linear scan over the segments vector occurs

  # Finding 8: range_header_value String allocation
  @finding-8 @perf @S
  Scenario: Range header value avoids heap allocation
    Given a segment range needs a header value
    When range_header_value is called
    Then the header string is written to a stack-allocated buffer
    And no heap allocation occurs for the format string
