Feature: Humanization
  As an hpx client user
  I want to generate human-like input events
  So that my requests pass behavioral fingerprinting

  Scenario: Generate mouse trajectory
    Given a BehaviorProfile with seed 42
    When I generate a mouse trajectory from (100, 100) to (500, 300) with target width 50
    Then the trajectory should have at least 10 points
    And the last point should be within 5 pixels of the target
    And all timestamps should be monotonically increasing

  Scenario: Generate keystroke timings
    Given a BehaviorProfile with seed 42
    When I generate keystroke timings for "hello world"
    Then I should get 11 timing entries
    And all dwell times should be positive
    And all flight times should be positive
    And timings should vary between keystrokes

  Scenario: Generate scroll burst
    Given a BehaviorProfile with seed 42 and Trackpad scroll style
    When I generate a scroll burst for 500 pixels
    Then I should get at least 3 wheel ticks
    And the total delta should approximate 500 pixels

  Scenario: Deterministic with same seed
    Given two BehaviorProfiles with the same seed
    When I generate mouse trajectories with both profiles
    Then the trajectories should be identical
