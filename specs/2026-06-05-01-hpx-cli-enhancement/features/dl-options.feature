Feature: hpx-cli Download Options
  As a user configuring downloads
  I want to set speed limits, checksums, mirrors, and other options
  So I have full control over download behavior

  Scenario: Download with speed limit
    Given the download engine is running
    And a file is available at "https://httpbin.org/bytes/10485760"
    When I run "hpx dl add https://httpbin.org/bytes/10485760 --speed-limit 1MB/s"
    Then the download is added successfully
    And the download speed is limited to approximately 1MB/s

  Scenario: Download with checksum verification
    Given the download engine is running
    And a file with known SHA-256 hash is available
    When I run "hpx dl add <url> --checksum sha256:<hash>"
    Then the download completes
    And the checksum is verified

  Scenario: Download with mirror URLs
    Given the download engine is running
    And primary URL fails
    And mirror URLs are available
    When I run "hpx dl add <primary_url> --mirror <mirror1> --mirror <mirror2>"
    Then the download falls back to a mirror URL

  Scenario: Download with custom headers
    Given the download engine is running
    And a file is available at "https://httpbin.org/headers"
    When I run "hpx dl add https://httpbin.org/headers -H 'Authorization:Bearer token'"
    Then the download request includes the custom header

  Scenario: Download with proxy
    Given the download engine is running
    And a proxy server is available
    When I run "hpx dl add <url> --proxy http://proxy:8080"
    Then the download proceeds through the proxy
