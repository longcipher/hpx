Feature: hpx-cli Form File Upload
  As a user uploading files via HTTP
  I want to use @filename syntax in form fields
  So I can upload file content easily

  Scenario: Upload file via form field
    Given the HTTP client is available
    And a file exists at "/tmp/test.txt" with content "hello"
    When I run "hpx -X POST httpbin.org/post --form file=@/tmp/test.txt"
    Then the request sends the file content as a form field
    And the response shows the uploaded file content

  Scenario: Upload multiple files via form fields
    Given the HTTP client is available
    And files exist at "/tmp/a.txt" and "/tmp/b.txt"
    When I run "hpx -X POST httpbin.org/post --form a=@/tmp/a.txt --form b=@/tmp/b.txt"
    Then the request sends both files as form fields
