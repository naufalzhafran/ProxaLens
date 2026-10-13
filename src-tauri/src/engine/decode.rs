//! Content-Encoding decoding (gzip, deflate, br, zstd) for display.
//!
//! Bodies are stored as they came over the wire and decoded lazily when the
//! UI asks for a flow. Truncated captures decode as far as the bytes allow.

use std::io::{self, Read};

/// Decoded output is capped so a small compressed body can't expand without
/// bound (a "zip bomb").
pub const DECODED_CAP: u64 = 50 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct Decoded {
    pub bytes: Vec<u8>,
    /// The encodings that were removed, as written in the header ("gzip, br").
    pub encoding: String,
    /// The input ended early or hit [`DECODED_CAP`]; `bytes` is a prefix.
    pub partial: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("unsupported content encoding \"{0}\"")]
    Unsupported(String),
    #[error("could not decode {encoding} body: {message}")]
    Corrupt { encoding: String, message: String },
}

/// Returns `Ok(None)` when no decoding is needed (no header, or identity).
pub fn decode(body: &[u8], content_encoding: Option<&str>) -> Result<Option<Decoded>, DecodeError> {
    let Some(header) = content_encoding else {
        return Ok(None);
    };
    // Encodings are listed in the order they were applied; undo in reverse.
    let codings: Vec<String> = header
        .split(',')
        .map(|c| c.trim().to_ascii_lowercase())
        .filter(|c| !c.is_empty() && c != "identity")
        .collect();
    if codings.is_empty() || body.is_empty() {
        return Ok(None);
    }

    let mut bytes = body.to_vec();
    let mut partial = false;
    for coding in codings.iter().rev() {
        let (out, p) = decode_one(&bytes, coding)?;
        bytes = out;
        partial |= p;
    }
    Ok(Some(Decoded {
        bytes,
        encoding: codings.join(", "),
        partial,
    }))
}

fn decode_one(input: &[u8], coding: &str) -> Result<(Vec<u8>, bool), DecodeError> {
    match coding {
        "gzip" | "x-gzip" => read_all(flate2::read::MultiGzDecoder::new(input), coding),
        // "deflate" is meant to be zlib-wrapped, but some servers send raw
        // deflate; try both.
        "deflate" => read_all(flate2::read::ZlibDecoder::new(input), coding)
            .or_else(|_| read_all(flate2::read::DeflateDecoder::new(input), coding)),
        "br" => read_all(brotli::Decompressor::new(input, 64 * 1024), coding),
        "zstd" => {
            let dec = zstd::stream::read::Decoder::new(input).map_err(|e| corrupt(coding, &e))?;
            read_all(dec, coding)
        }
        other => Err(DecodeError::Unsupported(other.to_string())),
    }
}

fn read_all(reader: impl Read, coding: &str) -> Result<(Vec<u8>, bool), DecodeError> {
    let mut out = Vec::new();
    let mut limited = reader.take(DECODED_CAP + 1);
    match limited.read_to_end(&mut out) {
        Ok(_) => {
            let capped = out.len() as u64 > DECODED_CAP;
            out.truncate(DECODED_CAP as usize);
            Ok((out, capped))
        }
        // A truncated capture ends mid-stream: keep what decoded so far.
        Err(e) if !out.is_empty() && is_truncation(&e) => Ok((out, true)),
        Err(e) => Err(corrupt(coding, &e)),
    }
}

fn is_truncation(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::UnexpectedEof | io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData
    )
}

fn corrupt(coding: &str, e: &io::Error) -> DecodeError {
    DecodeError::Corrupt {
        encoding: coding.to_string(),
        message: e.to_string(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    pub fn gzip(data: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    fn brotli_enc(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut w = brotli::CompressorWriter::new(&mut out, 4096, 5, 22);
            w.write_all(data).unwrap();
        }
        out
    }

    const TEXT: &[u8] = b"hello hello hello hello, compressed world";

    #[test]
    fn no_header_or_identity_is_untouched() {
        assert_eq!(decode(TEXT, None), Ok(None));
        assert_eq!(decode(TEXT, Some("identity")), Ok(None));
        assert_eq!(decode(b"", Some("gzip")), Ok(None));
    }

    #[test]
    fn decodes_each_encoding() {
        let zlib = {
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(TEXT).unwrap();
            e.finish().unwrap()
        };
        let raw_deflate = {
            let mut e =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(TEXT).unwrap();
            e.finish().unwrap()
        };
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("gzip", gzip(TEXT)),
            ("deflate", zlib),
            ("deflate", raw_deflate),
            ("br", brotli_enc(TEXT)),
            ("zstd", zstd::encode_all(TEXT, 3).unwrap()),
        ];
        for (enc, body) in cases {
            let d = decode(&body, Some(enc)).unwrap().unwrap();
            assert_eq!(d.bytes, TEXT, "{enc}");
            assert!(!d.partial, "{enc}");
        }
    }

    #[test]
    fn stacked_encodings_are_undone_in_reverse() {
        let body = brotli_enc(&gzip(TEXT));
        let d = decode(&body, Some("gzip, br")).unwrap().unwrap();
        assert_eq!(d.bytes, TEXT);
        assert_eq!(d.encoding, "gzip, br");
    }

    #[test]
    fn truncated_input_decodes_a_prefix() {
        let big: Vec<u8> = (0..200_000u32).flat_map(|i| i.to_le_bytes()).collect();
        let body = gzip(&big);
        let d = decode(&body[..body.len() / 2], Some("gzip"))
            .unwrap()
            .unwrap();
        assert!(d.partial);
        assert!(!d.bytes.is_empty());
        assert_eq!(d.bytes[..], big[..d.bytes.len()]);
    }

    #[test]
    fn reports_corrupt_and_unsupported() {
        assert!(matches!(
            decode(b"not gzip", Some("gzip")),
            Err(DecodeError::Corrupt { .. })
        ));
        assert_eq!(
            decode(TEXT, Some("compress")),
            Err(DecodeError::Unsupported("compress".into()))
        );
    }
}
