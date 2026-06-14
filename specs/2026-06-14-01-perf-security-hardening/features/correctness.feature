Feature: Correctness improvements for hpx-dl persistence
  As a user of the hpx download engine
  I want download progress to be reliably persisted
  So that interrupted downloads can be resumed without data loss

  # Finding 8 (renumbered): Silent persistence data loss on shutdown
  @finding-8 @correctness @M
  Scenario: Persistence worker drains pending commands on shutdown
    Given a persistence handle with pending Upsert commands
    When the engine is dropped
    Then the shutdown sequence waits for pending commands to be processed
    And no metadata is silently lost

  @finding-8 @correctness @M
  Scenario: Persistence worker acknowledges shutdown
    Given a persistence handle is active
    When a Shutdown command is sent
    Then the worker processes all queued commands before exiting
    And the drop implementation blocks briefly to drain the queue
