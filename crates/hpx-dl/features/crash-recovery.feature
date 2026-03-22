Feature: Crash Recovery
  As a developer using hpx-dl
  I want the engine to recover downloads after an unclean shutdown
  So that I don't lose download progress

  Scenario: Engine recovers in-progress downloads after crash
    Given a file "recovery.bin" of 5MB is available at "http://localhost:8080/recovery.bin"
    And the server supports range requests
    And an hpx-dl engine is configured with SQLite storage at "/tmp/hpx-dl-recovery.db"
    When I add a download for "http://localhost:8080/recovery.bin" to "recovery.bin"
    And the download reaches 30% progress
    And I simulate an unclean shutdown by dropping the engine without cleanup
    And I create a new engine with the same storage path "/tmp/hpx-dl-recovery.db"
    Then the download should be in state "Paused"
    When I resume the download
    And I wait for the download to complete
    Then the download state should be "Completed"
    And the file "recovery.bin" should be exactly 5242880 bytes
