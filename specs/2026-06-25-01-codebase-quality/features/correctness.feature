Feature: Correctness and Bug Fixes

  Fix real bugs in retry logic, redirect handling, compression safety,
  download scheduler, and queue operations.

  @finding-COR-02 @priority-high
  Scenario: Retry budget is preserved when request clone fails
    Given an HTTP client with a retry budget of 10 tokens
    And a request with a non-clonable streaming body
    When the request fails with a retryable error
    And tower calls retry() which signals Retryable
    And clone_request() returns None because the body cannot be cloned
    Then the retry budget should still have 10 tokens (not 9)
    And no retry should be attempted

  @finding-COR-02 @priority-high
  Scenario: Retry budget is withdrawn when retry succeeds
    Given an HTTP client with a retry budget of 10 tokens
    And a request with a clonable body
    When the request fails with a retryable error
    And tower calls retry() which signals Retryable
    And clone_request() returns the cloned request
    Then the retry budget should have 9 tokens
    And a retry should be attempted

  @finding-COR-04 @priority-high
  Scenario: Missing redirect policy returns error instead of panicking
    Given an HTTP client configured for redirect following
    And a request that triggers a redirect
    When the redirect middleware processes the response
    And the RequestConfig has no FollowRedirectPolicy set
    Then the client should return an error
    And the process should not panic

  @finding-COR-04 @priority-high
  Scenario: Normal redirect works when policy is present
    Given an HTTP client with redirect policy configured
    And a request that triggers a 302 redirect
    When the redirect middleware processes the response
    Then the client should follow the redirect
    And the final response should be returned successfully

  @finding-COR-05 @priority-medium
  Scenario: WebSocket compression uses safe buffer initialization
    Given a WebSocket compressor with permessage-deflate enabled
    When compressing a message payload
    Then no undefined behavior should occur during buffer allocation
    And the compressed output should be identical to the unsafe version

  @finding-COR-05 @priority-medium
  Scenario: Compressed roundtrip produces correct output
    Given a WebSocket compressor and decompressor
    When a text message "Hello, World!" is compressed
    And the compressed bytes are decompressed
    Then the decompressed output should equal "Hello, World!"

  @finding-COR-01 @priority-medium
  Scenario: Download added during scheduler shutdown is not stranded
    Given a download engine with max_concurrent = 2
    And one active download running
    When the scheduler finishes its last download and sets running = false
    And concurrently a new download is added to the queue
    And trigger_scheduler() is called
    Then the new download should be picked up within one scheduler cycle
    And should not remain permanently queued

  @finding-COR-09 @priority-medium
  Scenario: Removing a download from the queue completes quickly
    Given a download queue with 1000 entries
    When a download is removed by ID
    Then the operation should complete without allocating a temporary Vec
    And the remaining entries should maintain their priority ordering

  @finding-COR-09 @priority-medium
  Scenario: Queue ordering preserved after removal
    Given a download queue with entries at priorities [1, 5, 3, 10, 2]
    When the entry with priority 3 is removed
    Then popping entries should yield priorities [10, 5, 2, 1] in order
