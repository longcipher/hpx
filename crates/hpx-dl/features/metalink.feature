Feature: Metalink Download
  As a developer using hpx-dl
  I want to download files from Metalink manifests
  So that I can get multi-source downloads with automatic checksum verification

  Scenario: Download from Metalink v4 manifest
    Given a Metalink v4 file is available at "http://localhost:8080/file.metalink" containing:
      | file       | mirrors                                              | sha-256                                                            |
      | image.iso  | http://mirror1.example.com/image.iso, http://mirror2.example.com/image.iso | a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2 |
    And a file "image.iso" of 100MB is available at both mirrors
    And an hpx-dl engine is configured
    When I add a Metalink download from "http://localhost:8080/file.metalink"
    And I wait for the download to complete
    Then the download state should be "Completed"
    And the file "image.iso" should be exactly 104857600 bytes
