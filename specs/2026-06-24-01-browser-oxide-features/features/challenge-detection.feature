Feature: Challenge Detection
  As an hpx client user
  I want to detect anti-bot challenges from HTTP response bodies
  So that I can handle them appropriately

  Scenario: Detect Cloudflare managed challenge
    Given a response body containing "_cf_chl_opt" and "/cdn-cgi/challenge-platform/"
    When I classify the response body
    Then the classification tag should be "cf-managed"
    And the verdict should be ChallengeIncomplete

  Scenario: Detect DataDome captcha
    Given a response body containing "ddcaptchaencoded" under 30KB
    When I classify the response body
    Then the classification tag should be "datadome"
    And the verdict should be EdgeBlock

  Scenario: Detect AWS-WAF challenge
    Given a response body containing "token.awswaf.com" and "AwsWafIntegration" under 4KB
    When I classify the response body
    Then the classification tag should be "aws-waf"
    And the verdict should be ChallengeIncomplete

  Scenario: Detect Akamai sensor challenge
    Given a response body containing "akam/13" and "sensor_data" under 30KB
    When I classify the response body
    Then the classification tag should be "akamai-sensor"
    And the verdict should be SensorFail

  Scenario: Pass clean response
    Given a response body containing normal HTML content without challenge markers
    When I classify the response body
    Then the classification tag should be "l3-rendered"
    And the verdict should be Pass

  Scenario: Pass thin body without false positive
    Given a response body under 1KB with no challenge markers
    When I classify the response body
    Then the verdict should be RenderIncomplete
    And the tag should be "thin-body"
