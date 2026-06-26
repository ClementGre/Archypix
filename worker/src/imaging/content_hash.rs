//! Metadata-independent **content hash** (feature 11 §4).
//!
//! A SHA-256 over the image's *pixel-bearing* bytes with all metadata segments stripped, so the
//! hash is **stable across EXIF edits** (metadata-only rewrites leave the scan data untouched) and
//! **changes on a visual re-encode** (new scan data). It is computed from the result, never by
//! classifying the edit, so even a blind WebDAV PUT is handled the same way.
//!
//! Format coverage:
//! - **JPEG** — strip `APPn` (EXIF/XMP/ICC/JFIF) and `COM` segments; hash the framing segments
//!   (DQT/SOF/DHT/…) plus the entropy-coded scan.
//! - **PNG** — strip ancillary text/time chunks (`tEXt`/`zTXt`/`iTXt`/`eXIf`/`tIME`); hash the
//!   rest (`IHDR`/`PLTE`/`IDAT`/…).
//! - **Anything else** — `None`; the dedup reconciler falls back to `file_hash` for grouping.
//!
//! The result is **deterministic across instances** — it is purely a function of the bytes, with no
//! decoder in the loop — which matters because copies across a share graph are hashed by different
//! backends.

use sha2::{Digest, Sha256};
use std::path::Path;
use tracing::{debug, instrument};

/// JPEG: skip `APP0`–`APP15` (`0xE0`–`0xEF`) and `COM` (`0xFE`) segments.
fn is_jpeg_metadata_marker(marker: u8) -> bool {
    (0xE0..=0xEF).contains(&marker) || marker == 0xFE
}

/// PNG ancillary chunks that carry only metadata (stripped before hashing).
const PNG_METADATA_CHUNKS: [&[u8; 4]; 5] = [b"tEXt", b"zTXt", b"iTXt", b"eXIf", b"tIME"];

const JPEG_SOI: [u8; 2] = [0xFF, 0xD8];
const PNG_SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Compute the metadata-stripped content hash of the file at `path`.
///
/// Returns `None` for a format we cannot strip (the caller falls back to `file_hash`). Reads the
/// whole file into memory — acceptable since `gen_thumbnail` already decodes the image. Must be
/// called inside `tokio::task::spawn_blocking`.
#[instrument(skip(path), fields(file = ?path.file_name()))]
pub fn content_hash(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let hash = content_hash_bytes(&bytes);
    debug!(stripped = hash.is_some(), "content hash computed");
    hash
}

/// The metadata-stripped hash of an in-memory image, or `None` for an unsupported format.
pub fn content_hash_bytes(bytes: &[u8]) -> Option<String> {
    if bytes.starts_with(&JPEG_SOI) {
        jpeg_content_hash(bytes)
    } else if bytes.starts_with(&PNG_SIG) {
        png_content_hash(bytes)
    } else {
        None
    }
}

/// Hash a JPEG's framing + scan, skipping `APPn`/`COM` metadata segments.
fn jpeg_content_hash(bytes: &[u8]) -> Option<String> {
    let mut hasher = Sha256::new();
    let len = bytes.len();
    // Past the SOI marker (already checked).
    let mut pos = 2usize;
    loop {
        // Need at least a 2-byte marker.
        if pos + 2 > len {
            break;
        }
        // Markers are byte-aligned `0xFF <code>`; fill bytes (`0xFF` padding) are tolerated.
        if bytes[pos] != 0xFF {
            // Malformed stream — fold the remainder in so a truncated/odd file still hashes stably.
            hasher.update(&bytes[pos..]);
            break;
        }
        let marker = bytes[pos + 1];
        pos += 2;

        match marker {
            0xFF => {
                // Padding byte before the real marker; back up one and retry.
                pos -= 1;
                continue;
            }
            // Standalone markers (no length, no payload): RSTn, TEM.
            0xD0..=0xD9 | 0x01 => continue,
            // Start of Scan: header + entropy-coded data run to the end. Metadata never appears
            // after the scan in the files we care about, so hashing to EOF is both correct and
            // re-encode-sensitive.
            0xDA => {
                hasher.update(&bytes[pos - 2..]);
                break;
            }
            _ => {
                // Segment with a 2-byte big-endian length (the length count includes itself).
                if pos + 2 > len {
                    break;
                }
                let seg_len = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as usize;
                if seg_len < 2 {
                    break;
                }
                let seg_end = pos + seg_len;
                if seg_end > len {
                    break;
                }
                if !is_jpeg_metadata_marker(marker) {
                    // Include the marker, length and payload of a framing segment (DQT/SOF/DHT/…).
                    hasher.update(&bytes[pos - 2..seg_end]);
                }
                pos = seg_end;
            }
        }
    }
    Some(format!("{:x}", hasher.finalize()))
}

/// Hash a PNG's critical/visual chunks, skipping ancillary text/time chunks.
fn png_content_hash(bytes: &[u8]) -> Option<String> {
    let mut hasher = Sha256::new();
    let len = bytes.len();
    let mut pos = PNG_SIG.len();
    loop {
        // Each chunk: length(4) + type(4) + data + crc(4).
        if pos + 8 > len {
            break;
        }
        let data_len =
            u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
                as usize;
        let ctype = &bytes[pos + 4..pos + 8];
        let data_start = pos + 8;
        let data_end = data_start + data_len;
        // Bounds check including the trailing CRC.
        if data_end + 4 > len {
            break;
        }
        let is_metadata = PNG_METADATA_CHUNKS.iter().any(|m| m.as_slice() == ctype);
        if !is_metadata {
            hasher.update(ctype);
            hasher.update(&bytes[data_start..data_end]);
        }
        let is_end = ctype == b"IEND";
        pos = data_end + 4; // skip CRC
        if is_end {
            break;
        }
    }
    Some(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal JPEG-shaped byte stream: SOI, the given segments (marker + payload), then an
    /// SOS marker with the given scan bytes.
    fn jpeg(segments: &[(u8, &[u8])], scan: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8];
        for (marker, payload) in segments {
            v.push(0xFF);
            v.push(*marker);
            let seg_len = (payload.len() + 2) as u16;
            v.extend_from_slice(&seg_len.to_be_bytes());
            v.extend_from_slice(payload);
        }
        v.push(0xFF);
        v.push(0xDA); // SOS
        v.extend_from_slice(&2u16.to_be_bytes()); // empty SOS header
        v.extend_from_slice(scan);
        v
    }

    #[test]
    fn jpeg_ignores_app_segments() {
        // Two JPEGs identical except for their APP1 (EXIF) payload — content hash must match.
        let dqt: &[u8] = &[0x00, 0x01, 0x02, 0x03];
        let a = jpeg(&[(0xE1, b"EXIF-AAAA"), (0xDB, dqt)], b"scandata");
        let b = jpeg(
            &[(0xE1, b"EXIF-DIFFERENT-LENGTH"), (0xDB, dqt)],
            b"scandata",
        );
        assert_eq!(content_hash_bytes(&a), content_hash_bytes(&b));
    }

    #[test]
    fn jpeg_scan_change_changes_hash() {
        let dqt: &[u8] = &[0x00, 0x01];
        let a = jpeg(&[(0xDB, dqt)], b"scan-one");
        let b = jpeg(&[(0xDB, dqt)], b"scan-two");
        assert_ne!(content_hash_bytes(&a), content_hash_bytes(&b));
    }

    #[test]
    fn jpeg_framing_change_changes_hash() {
        // A different quantization table (DQT) is a visual change → different hash.
        let a = jpeg(&[(0xDB, &[0x00, 0x01])], b"scan");
        let b = jpeg(&[(0xDB, &[0x09, 0x09])], b"scan");
        assert_ne!(content_hash_bytes(&a), content_hash_bytes(&b));
    }

    /// Build a minimal PNG-shaped stream from chunks (type, data); CRC bytes are filler.
    fn png(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut v = PNG_SIG.to_vec();
        for (ctype, data) in chunks {
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(ctype.as_slice());
            v.extend_from_slice(data);
            v.extend_from_slice(&[0, 0, 0, 0]); // CRC (ignored by the hasher)
        }
        v
    }

    #[test]
    fn png_ignores_text_chunks() {
        let ihdr: &[u8] = &[1, 2, 3, 4];
        let idat: &[u8] = &[9, 9, 9];
        let a = png(&[
            (b"IHDR", ihdr),
            (b"tEXt", b"Comment=hello"),
            (b"IDAT", idat),
            (b"IEND", b""),
        ]);
        let b = png(&[
            (b"IHDR", ihdr),
            (b"tEXt", b"Comment=totally-different"),
            (b"IDAT", idat),
            (b"IEND", b""),
        ]);
        assert_eq!(content_hash_bytes(&a), content_hash_bytes(&b));
    }

    #[test]
    fn png_idat_change_changes_hash() {
        let ihdr: &[u8] = &[1, 2, 3, 4];
        let a = png(&[(b"IHDR", ihdr), (b"IDAT", &[1, 1, 1]), (b"IEND", b"")]);
        let b = png(&[(b"IHDR", ihdr), (b"IDAT", &[2, 2, 2]), (b"IEND", b"")]);
        assert_ne!(content_hash_bytes(&a), content_hash_bytes(&b));
    }

    #[test]
    fn unsupported_format_is_none() {
        assert_eq!(content_hash_bytes(b"GIF89a not an image we strip"), None);
        assert_eq!(content_hash_bytes(&[]), None);
    }
}
