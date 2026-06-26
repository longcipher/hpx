use std::io::{self, Read, Result, Write};

use boring::ssl::{CertificateCompressionAlgorithm, CertificateCompressor};
use brotli::{CompressorWriter, Decompressor};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
// use zstd::stream::{Decoder as ZstdDecoder, Encoder as ZstdEncoder};

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct BrotliCertificateCompressor;

impl CertificateCompressor for BrotliCertificateCompressor {
    const ALGORITHM: CertificateCompressionAlgorithm = CertificateCompressionAlgorithm::BROTLI;
    const CAN_COMPRESS: bool = true;
    const CAN_DECOMPRESS: bool = true;

    fn compress<W>(&self, input: &[u8], output: &mut W) -> Result<()>
    where
        W: Write,
    {
        let mut writer = CompressorWriter::new(output, input.len(), 11, 22);
        writer.write_all(input)?;
        writer.flush()?;
        Ok(())
    }

    fn decompress<W>(&self, input: &[u8], output: &mut W) -> Result<()>
    where
        W: Write,
    {
        let mut reader = Decompressor::new(input, 4096);
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf[..]) {
                Err(e) => {
                    if let io::ErrorKind::Interrupted = e.kind() {
                        continue;
                    }
                    return Err(e);
                }
                Ok(size) => {
                    if size == 0 {
                        break;
                    }
                    output.write_all(&buf[..size])?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ZlibCertificateCompressor;

impl CertificateCompressor for ZlibCertificateCompressor {
    const ALGORITHM: CertificateCompressionAlgorithm = CertificateCompressionAlgorithm::ZLIB;
    const CAN_COMPRESS: bool = true;
    const CAN_DECOMPRESS: bool = true;

    fn compress<W>(&self, input: &[u8], output: &mut W) -> Result<()>
    where
        W: Write,
    {
        let mut encoder = ZlibEncoder::new(output, Compression::default());
        encoder.write_all(input)?;
        encoder.finish()?;
        Ok(())
    }

    fn decompress<W>(&self, input: &[u8], output: &mut W) -> Result<()>
    where
        W: Write,
    {
        let mut decoder = ZlibDecoder::new(input);
        io::copy(&mut decoder, output)?;
        Ok(())
    }
}

// ponytail: ZstdCertificateCompressor omitted — boring crate lacks CertificateCompressionAlgorithm::ZSTD.
// Uncomment when boring adds the ZSTD variant. The compressor logic is already written below.
//
// #[derive(Debug, Clone, Default)]
// #[non_exhaustive]
// pub struct ZstdCertificateCompressor;
//
// impl CertificateCompressor for ZstdCertificateCompressor {
//     const ALGORITHM: CertificateCompressionAlgorithm = CertificateCompressionAlgorithm::ZSTD;
//     const CAN_COMPRESS: bool = true;
//     const CAN_DECOMPRESS: bool = true;
//
//     fn compress<W>(&self, input: &[u8], output: &mut W) -> Result<()>
//     where
//         W: Write,
//     {
//         let mut writer = ZstdEncoder::new(output, 0)?;
//         writer.write_all(input)?;
//         writer.flush()?;
//         Ok(())
//     }
//
//     fn decompress<W>(&self, input: &[u8], output: &mut W) -> Result<()>
//     where
//         W: Write,
//     {
//         let mut reader = ZstdDecoder::new(input)?;
//         let mut buf = [0u8; 4096];
//         loop {
//             match reader.read(&mut buf[..]) {
//                 Err(e) => {
//                     if let io::ErrorKind::Interrupted = e.kind() {
//                         continue;
//                     }
//                     return Err(e);
//                 }
//                 Ok(size) => {
//                     if size == 0 {
//                         break;
//                     }
//                     output.write_all(&buf[..size])?;
//                 }
//             }
//         }
//         Ok(())
//     }
// }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brotli_round_trip() {
        let compressor = BrotliCertificateCompressor;
        let input = b"Hello, TLS certificate compression with Brotli!";
        let mut compressed = Vec::new();
        compressor.compress(input, &mut compressed).unwrap();
        assert!(!compressed.is_empty());

        let mut decompressed = Vec::new();
        compressor
            .decompress(&compressed, &mut decompressed)
            .unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn zlib_round_trip() {
        let compressor = ZlibCertificateCompressor;
        let input = b"Hello, TLS certificate compression with Zlib!";
        let mut compressed = Vec::new();
        compressor.compress(input, &mut compressed).unwrap();
        assert!(!compressed.is_empty());

        let mut decompressed = Vec::new();
        compressor
            .decompress(&compressed, &mut decompressed)
            .unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn brotli_empty_input() {
        let compressor = BrotliCertificateCompressor;
        let mut compressed = Vec::new();
        compressor.compress(b"", &mut compressed).unwrap();

        let mut decompressed = Vec::new();
        compressor
            .decompress(&compressed, &mut decompressed)
            .unwrap();
        assert_eq!(decompressed, b"");
    }

    #[test]
    fn zlib_empty_input() {
        let compressor = ZlibCertificateCompressor;
        let mut compressed = Vec::new();
        compressor.compress(b"", &mut compressed).unwrap();

        let mut decompressed = Vec::new();
        compressor
            .decompress(&compressed, &mut decompressed)
            .unwrap();
        assert_eq!(decompressed, b"");
    }
}
