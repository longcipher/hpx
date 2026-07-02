Feature: DomElement safe access
  As a developer touching the DOM layer
  I want to eliminate unreachable!() panic paths in DomElement
  So that DOM access is panic-free and uses expect() for明确的安全不变量

  Background:
    Given the DomElement wrapper in dom.rs currently uses `unreachable!()` in two accessor methods

  Scenario: node() uses expect with safety message instead of unreachable!
    When I access DomElement::node()
    Then if the invariant is violated the panic message includes "validated in constructor"

  Scenario: element_data() uses expect with safety message instead of unreachable!
    When I access DomElement::element_data()
    Then if the invariant is violated the panic message includes "validated in constructor"

  Scenario: All existing DomElement callers compile and work
    When I use DomElement in the layout engine and css_selector matching
    Then behavior is identical to the previous implementation
