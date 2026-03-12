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

/// Compute the hex-encoded SHA-256 digest of an in-memory byte slice.
pub fn compute_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
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
