use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn hash_file_known_bytes() {
        // SHA-256("hello\n") == 5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(b"hello\n").unwrap();
        tmp.flush().unwrap();

        let digest = hash_file(tmp.path());
        assert_eq!(
            digest,
            Some("5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03".into()),
            "SHA-256(b\"hello\\n\") must match"
        );
    }

    #[test]
    fn hash_file_empty() {
        // SHA-256("") == e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let digest = hash_file(tmp.path());
        assert_eq!(
            digest,
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into())
        );
    }
}

/// Compute the SHA-256 hex digest of the file at `path`.
///
/// Reads in 64 KiB chunks to avoid buffering large files in memory.
/// Must be called inside `tokio::task::spawn_blocking`.
pub fn hash_file(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(format!("{:x}", hasher.finalize()))
}

pub fn hash_bytes(bytes: &Vec<u8>) -> Option<String> {
    let mut hasher = Sha256::new();
    let mut offset = 0;
    let buf_len = 65526;
    while offset < bytes.len() {
        let n = std::cmp::min(buf_len, bytes.len() - offset);
        hasher.update(&bytes[offset..offset + n]);
        offset += n;
    }
    Some(format!("{:x}", hasher.finalize()))
}
