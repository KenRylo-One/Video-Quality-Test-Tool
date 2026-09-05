//! File hashes.

use crate::error::{BackendError, Result};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use vqtt_core::media::{FINGERPRINT_CHUNK_BYTES, Fingerprint};

/// Reads a whole file and returns its SHA-256 hash, as lower case hexadecimal.
///
/// The tool uses this for a back-end binary, which is small. It never uses it for a
/// video file.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|source| BackendError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| BackendError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Builds the quick fingerprint of a media file.
///
/// A SHA-256 hash of a 50 GB reference costs minutes on every run. This reads the size,
/// the first 8 MiB and the last 8 MiB instead.
pub fn fingerprint_file(path: &Path) -> Result<Fingerprint> {
    let mut file = File::open(path).map_err(|source| BackendError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let bytes = file
        .metadata()
        .map_err(|source| BackendError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .len();

    let chunk = FINGERPRINT_CHUNK_BYTES.min(bytes) as usize;
    let mut head_buffer = vec![0_u8; chunk];
    file.read_exact(&mut head_buffer)
        .map_err(|source| BackendError::Io {
            path: path.to_path_buf(),
            source,
        })?;

    let tail_start = bytes.saturating_sub(FINGERPRINT_CHUNK_BYTES);
    file.seek(SeekFrom::Start(tail_start))
        .map_err(|source| BackendError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let mut tail_buffer = Vec::with_capacity(chunk);
    file.read_to_end(&mut tail_buffer)
        .map_err(|source| BackendError::Io {
            path: path.to_path_buf(),
            source,
        })?;

    Ok(Fingerprint {
        bytes,
        head: hex(&Sha256::digest(&head_buffer)),
        tail: hex(&Sha256::digest(&tail_buffer)),
    })
}

/// Writes bytes as lower case hexadecimal.
fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn hashes_a_known_string() {
        let dir = std::env::temp_dir().join("vqtt-hash-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("abc.txt");
        let mut file = File::create(&path).unwrap();
        file.write_all(b"abc").unwrap();
        drop(file);

        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_short_file_hashes_both_ends_over_the_same_bytes() {
        let dir = std::env::temp_dir().join("vqtt-hash-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("short.bin");
        std::fs::write(&path, b"0123456789").unwrap();

        let print = fingerprint_file(&path).unwrap();
        assert_eq!(print.bytes, 10);
        assert_eq!(print.head, print.tail);
        assert!(print.to_string().starts_with("qf1:10:"));
        std::fs::remove_file(&path).ok();
    }
}
