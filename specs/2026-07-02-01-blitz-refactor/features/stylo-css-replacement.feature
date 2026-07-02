Feature: Stylo-based CSS engine absorbing Blitz's approach
  As a developer maintaining hpx-browser
  I want to evaluate Stylo as a replacement for the custom CSS parser/cascade stack
  So that CSS behavior matches Firefox/spec exactly, eliminating a whole class of bot-detection signals

  Background:
    Given hpx-browser currently has ~4600 lines of custom CSS parser/cascade/selectors/values code

  Scenario: Stylo parses real-world stylesheets correctly
    When I feed a production CSS stylesheet (e.g., Bootstrap) to the Stylo integration
    Then it produces the same computed style rules as Firefox for common selectors

  Scenario: Computed style can be queried for CDP getComputedStyle
    When a CDP client calls Runtime.evaluate requesting getComputedStyle on an element
    Then the returned values match what the layout engine has computed

  Scenario: Inline style attributes still work
    When an element has a `style="color: red; font-size: 14px"` attribute
    Then the cascade correctly applies these as author-origin inline styles
    And they override stylesheet rules of lower specificity
