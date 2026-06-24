Feature: CSP Parsing
  As an hpx client user
  I want to parse and enforce Content Security Policy headers
  So that I can comply with server security policies

  Scenario: Parse simple CSP policy
    Given a CSP header "default-src 'self'; script-src 'self' 'unsafe-inline'"
    When I parse the policy
    Then the policy should have 2 directives
    And default-src should contain Self_
    And script-src should contain Self_ and UnsafeInline

  Scenario: Parse strict-dynamic policy
    Given a CSP header "script-src 'strict-dynamic' 'nonce-abc123' https://cdn.example.com"
    When I parse the policy
    And I check a script from "https://cdn.example.com" without nonce
    Then the script should be blocked
    And I check a script with nonce "abc123"
    Then the script should be allowed

  Scenario: Parse nonce and hash sources
    Given a CSP header "script-src 'nonce-test' 'sha256-abc123'"
    When I parse the policy
    Then script-src should contain a nonce source "test"
    And script-src should contain a SHA-256 hash source

  Scenario: Default-src fallback
    Given a CSP header "default-src 'self'"
    When I check an image from "https://img.example.com"
    Then the image should be blocked
    And I check an image from the same origin
    Then the image should be allowed

  Scenario: Report-only policy
    Given a report-only CSP header "script-src 'self'"
    When I parse the policy
    Then the policy should be marked as report-only
    And the allows check should return report_only=true

  Scenario: Parse multiple policies
    Given two CSP headers "default-src 'self'" and "script-src 'unsafe-inline'"
    When I parse both policies into a PolicySet
    Then the PolicySet should have 2 policies
    And a script with 'unsafe-inline' should be blocked (first policy lacks it)
