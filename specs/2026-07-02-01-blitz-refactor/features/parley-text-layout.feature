Feature: Parley text layout integration from Blitz
  As a developer maintaining hpx-browser canvas rendering
  I want to evaluate Parley as the text layout engine
  So that text metrics match real browsers (reducing fingerprinting surface)

  Background:
    Given hpx-browser canvas feature currently uses rustybuzz + swash directly

  Scenario: Parley produces the same glyph positions as rustybuzz for Latin text
    When I render "Hello, World!" with the same font and size through both engines
    Then the glyph positions and advances match within 0.5px

  Scenario: Canvas feature compiles with Parley as the layout backend
    When the `canvas` feature is enabled with the new parley integration
    Then `cargo build -p hpx-browser --features canvas` succeeds
    And text rendering produces visually equivalent output
