//! SHA-256 hashing utilities used for download verification.

use sha2::{Digest, Sha256};
use std::io::{self, Write};

/// A [`Write`] adapter that transparently computes a SHA-256 digest of all
/// bytes written to the underlying writer.
pub struct ChecksumWriter<W: Write> {
    inner: W,
    hasher: Sha256,
}

impl<W: Write> ChecksumWriter<W> {
    /// Wrap `inner` in a new `ChecksumWriter`.
    pub fn new(inner: W) -> Self {
        ChecksumWriter {
            inner,
            hasher: Sha256::new(),
        }
    }

    /// Consume the writer and return the inner writer together with the hex-encoded SHA-256 digest.
    pub fn finalize(self) -> (W, String) {
        let hash = hex::encode(self.hasher.finalize());
        (self.inner, hash)
    }
}

impl<W: Write> Write for ChecksumWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Compute SHA256 of a file on disk by streaming it.
pub fn sha256_file(path: &std::path::Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    // Known SHA-256 of the empty string:
    // e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    // Known SHA-256 of b"hello":
    // 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
    const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    // ── ChecksumWriter ────────────────────────────────────────────────────────

    #[test]
    fn checksum_writer_empty_input() {
        let buf: Vec<u8> = Vec::new();
        let mut writer = ChecksumWriter::new(buf);
        writer.flush().unwrap();
        let (_, hash) = writer.finalize();
        assert_eq!(hash, EMPTY_SHA256);
    }

    #[test]
    fn checksum_writer_known_input() {
        let buf: Vec<u8> = Vec::new();
        let mut writer = ChecksumWriter::new(buf);
        writer.write_all(b"hello").unwrap();
        let (inner, hash) = writer.finalize();
        assert_eq!(hash, HELLO_SHA256);
        assert_eq!(inner, b"hello");
    }

    #[test]
    fn checksum_writer_multi_chunk() {
        let buf: Vec<u8> = Vec::new();
        let mut writer = ChecksumWriter::new(buf);
        writer.write_all(b"hel").unwrap();
        writer.write_all(b"lo").unwrap();
        let (_, hash) = writer.finalize();
        assert_eq!(hash, HELLO_SHA256);
    }

    // ── sha256_file ───────────────────────────────────────────────────────────

    #[test]
    fn sha256_file_empty_file() {
        let mut f = NamedTempFile::new().unwrap();
        f.flush().unwrap();
        let hash = sha256_file(f.path()).unwrap();
        assert_eq!(hash, EMPTY_SHA256);
    }

    #[test]
    fn sha256_file_known_content() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"hello").unwrap();
        f.flush().unwrap();
        let hash = sha256_file(f.path()).unwrap();
        assert_eq!(hash, HELLO_SHA256);
    }

    #[test]
    fn sha256_file_missing_path_returns_error() {
        let result = sha256_file(std::path::Path::new("/nonexistent/path/to/file"));
        assert!(result.is_err());
    }
}
