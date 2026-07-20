Feature: HTTP/1.1 RFC Gap Closure
  As a high-performance HTTP client
  I want full RFC 7230/9110/9112 compliance for HTTP/1.1
  So that I correctly handle trailers, 1xx responses, Expect: 100-continue, and reject request smuggling

  Background:
    Given a local h1 test server is running on `127.0.0.1:8080`
    And a `Client` built with default configuration

  # REQ-15, REQ-18
  # C-19, C-20

  Scenario: HTTP/1.1 trailers are parsed
    When I send a GET to `https://127.0.0.1:8080/with-trailers`
    And the server responds with chunked body and trailers `X-Checksum: abc123` and `X-Final-Status: ok`
    Then `Response::trailers()` returns a `HeaderMap` containing `X-Checksum: abc123`
    And `Response::trailers()` returns a `HeaderMap` containing `X-Final-Status: ok`

  Scenario: HTTP/1.1 trailers announced via Trailer header
    When I send a GET to `https://127.0.0.1:8080/with-trailers`
    And the server responds with `Trailer: X-Checksum` followed by chunked body and `X-Checksum: abc123` trailer
    Then the client does not surface `X-Checksum` as a regular response header
    And the client surfaces `X-Checksum` only via `Response::trailers()`

  Scenario: HTTP/1.1 1xx informational responses are surfaced
    When I send a GET to `https://127.0.0.1:8080/early-hints`
    And the server responds with `103 Early Hints` (containing `Link: </style.css>; rel=preload`) followed by `200 OK`
    Then the client surfaces the `103` response via `Response::informational()`
    And the client surfaces the `200` response as the final response
    And the `103` response's `Link` header is accessible

  Scenario: Expect: 100-continue waits for 100 response
    Given a request with `Expect: 100-continue` header and a body of 1 MiB
    When the server responds with `100 Continue` after 100ms
    Then the client sends the body only after receiving `100 Continue`
    And the body is sent in full

  Scenario: Expect: 100-continue aborts on 4xx
    Given a request with `Expect: 100-continue` header and a body of 1 MiB
    When the server responds with `417 Expectation Failed` without sending `100 Continue`
    Then the client does NOT send the body
    And the response status `417` is surfaced to the application

  Scenario: Expect: 100-continue times out and sends body anyway
    Given a request with `Expect: 100-continue` header and a body of 1 MiB
    And the `ExpectContinueTimeout` is `1 second`
    When the server does not respond within 1 second
    Then the client sends the body anyway after the timeout

  Scenario: Request smuggling rejected
    When the server receives a request with both `Transfer-Encoding: chunked` and `Content-Length: 5`
    Then the client's outgoing request pipeline rejects this combination
    And an error is surfaced indicating ambiguous framing

  Scenario: Malformed chunked extensions rejected
    When the server responds with a chunked body containing a malformed chunk extension `5\r\nfoo; bad=extension with space\r\n`
    Then the client rejects the response
    And an error is surfaced indicating malformed chunked encoding

  Scenario: Obsolete line folding rejected
    When the server responds with a header value containing obs-fold (`CRLF SP` continuation)
    Then the client rejects the response
    And an error is surfaced indicating obs-fold is deprecated per RFC 9112 §2.3

  Scenario: HTTP/1.1 pipeline with 1xx and trailers works end-to-end
    When I send a GET to `https://127.0.0.1:8080/complex`
    And the server responds with `103 Early Hints`, then `200 OK` with chunked body and trailers
    Then the client surfaces `103` via `informational()`
    And the client surfaces `200` as the final response
    And `Response::trailers()` returns the trailers
