//! Checking what was downloaded before it's used: its SHA-256, and an unpacked archive
//! holding nothing but folders and ordinary files.

use anyhow::Result;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

/// The file's SHA-256, in lowercase hex.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Refuse anything under `dir` that isn't a folder or an ordinary file (a link could
/// point outside it).
pub fn only_plain_files(dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            only_plain_files(&entry.path())?;
        } else if !kind.is_file() {
            anyhow::bail!("the archive holds {}, which isn't an ordinary file", entry.path().display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_sum_and_links_refused() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a");
        std::fs::write(&f, "abc").unwrap();
        assert_eq!(sha256_file(&f).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        assert!(only_plain_files(dir.path()).is_ok());
        std::os::unix::fs::symlink("/etc/passwd", dir.path().join("sub/link")).unwrap();
        assert!(only_plain_files(dir.path()).is_err());
    }
}
