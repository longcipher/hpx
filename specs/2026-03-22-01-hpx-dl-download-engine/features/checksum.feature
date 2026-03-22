Feature: Checksum Verification
  As a developer using hpx-dl
  I want downloads to be verified with checksums
  So that I know the downloaded file is not corrupted

  Scenario: Correct checksum passes verification
    Given a file "verified.bin" with SHA-256 hash "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855" is available at "http://localhost:8080/verified.bin"
    And an hpx-dl engine is configured
    When I add a download for "http://localhost:8080/verified.bin" to "verified.bin" with SHA-256 checksum "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    And I wait for the download to complete
    Then the download state should be "Completed"

  Scenario: Incorrect checksum fails verification
    Given a file "corrupt.bin" is available at "http://localhost:8080/corrupt.bin"
    And an hpx-dl engine is configured
    When I add a download for "http://localhost:8080/corrupt.bin" to "corrupt.bin" with SHA-256 checksum "0000000000000000000000000000000000000000000000000000000000000000"
    And I wait for the download to complete
    Then the download state should be "Failed"
    And the download error should contain "Checksum mismatch"
