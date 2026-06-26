Feature: TLS Certificate Compression
  As an HTTP client user emulating browsers
  I want TLS 1.3 certificate compression matching real browsers
  So that my TLS fingerprints are accurate

  Scenario: Brotli compressor round-trips correctly
    Given a BrotliCompressor instance
    When I compress and then decompress sample certificate data
    Then the output should match the original input

  Scenario: Zlib compressor round-trips correctly
    Given a ZlibCompressor instance
    When I compress and then decompress sample certificate data
    Then the output should match the original input

  Scenario: Zstd compressor round-trips correctly
    Given a ZstdCompressor instance
    When I compress and then decompress sample certificate data
    Then the output should match the original input

  Scenario: Chrome profiles use Brotli compression only
    When I create a Chrome147 emulation with compression enabled
    Then only BrotliCompressor should be registered

  Scenario: Firefox profiles use all three compressors
    When I create a Firefox136 emulation with compression enabled
    Then BrotliCompressor, ZlibCompressor, and ZstdCompressor should all be registered

  Scenario: Safari profiles use all three compressors
    When I create a Safari18 emulation with compression enabled
    Then BrotliCompressor, ZlibCompressor, and ZstdCompressor should all be registered
