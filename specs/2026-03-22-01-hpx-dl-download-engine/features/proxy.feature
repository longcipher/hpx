Feature: Proxy Download
  As a developer using hpx-dl
  I want to download files through a proxy
  So that I can work behind corporate firewalls or for privacy

  Scenario: Download through HTTP proxy
    Given an HTTP proxy is running at "http://localhost:3128"
    And a file "proxied.bin" of 1MB is available at "http://localhost:8080/proxied.bin"
    And an hpx-dl engine is configured with proxy "http://localhost:3128"
    When I add a download for "http://localhost:8080/proxied.bin" to "proxied.bin"
    And I wait for the download to complete
    Then the download state should be "Completed"
    And the file "proxied.bin" should be exactly 1048576 bytes
