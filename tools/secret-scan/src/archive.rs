//! Read the single regular executable from a verified, bounded USTAR archive.
use super::process::Result;
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::Path,
};
fn number(bytes: &[u8]) -> Result<u64> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "The verified archive has invalid numeric metadata.")?
        .trim_matches(['\0', ' ']);
    u64::from_str_radix(text, 8).map_err(|_| "The verified archive has invalid numeric metadata.")
}
pub fn extract(tar: &Path, binary: &Path) -> Result<()> {
    let mut source =
        File::open(tar).map_err(|_| "The verified scanner archive could not be opened.")?;
    let mut found = false;
    loop {
        let mut header = [0u8; 512];
        source
            .read_exact(&mut header)
            .map_err(|_| "The verified scanner archive is truncated.")?;
        if header.iter().all(|b| *b == 0) {
            break;
        }
        let expected = number(&header[148..156])?;
        let sum: u64 = header
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if (148..156).contains(&i) {
                    u64::from(b' ')
                } else {
                    u64::from(*b)
                }
            })
            .sum();
        if sum != expected {
            return Err("The verified scanner archive header is invalid.");
        }
        let name = &header[..100];
        let name = &name[..name.iter().position(|b| *b == 0).unwrap_or(name.len())];
        let size = number(&header[124..136])?;
        if size > 64 * 1024 * 1024 {
            return Err("The verified scanner archive member exceeds its bound.");
        }
        if name == b"gitleaks" && header[345..500].iter().all(|b| *b == 0) {
            if found || !matches!(header[156], 0 | b'0') {
                return Err(
                    "The verified archive does not contain one regular scanner executable.",
                );
            }
            let mut bytes = Vec::new();
            let copied = (&mut source)
                .take(size)
                .read_to_end(&mut bytes)
                .map_err(|_| "The verified executable could not be read.")?;
            if copied as u64 != size {
                return Err("The verified executable is truncated.");
            }
            fs::write(binary, bytes)
                .map_err(|_| "The verified executable could not be written.")?;
            found = true;
        } else {
            source
                .seek(SeekFrom::Current(
                    i64::try_from(size).map_err(|_| "Invalid scanner archive size.")?,
                ))
                .map_err(|_| "The verified archive could not be traversed.")?;
        }
        let padding = (512 - size % 512) % 512;
        source
            .seek(SeekFrom::Current(padding as i64))
            .map_err(|_| "The verified archive padding is invalid.")?;
    }
    if found {
        Ok(())
    } else {
        Err("The verified archive does not contain the expected executable.")
    }
}

#[cfg(test)]
mod tests {
    use super::super::process::PrivateDir;
    use super::*;

    fn member(kind: u8) -> Vec<u8> {
        let mut bytes = vec![0; 1024];
        bytes[..8].copy_from_slice(b"gitleaks");
        bytes[124..136].copy_from_slice(b"00000000004\0");
        bytes[148..156].fill(b' ');
        bytes[156] = kind;
        let checksum: u64 = bytes[..512].iter().map(|b| u64::from(*b)).sum();
        bytes[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
        bytes[512..516].copy_from_slice(b"test");
        bytes
    }

    #[test]
    fn scanner_archive_requires_one_complete_regular_executable() {
        let dir = PrivateDir::new(&std::env::temp_dir()).unwrap();
        let archive = dir.0.join("scanner.tar");
        let binary = dir.0.join("scanner");
        let mut valid = member(b'0');
        valid.extend([0; 1024]);
        fs::write(&archive, &valid).unwrap();
        extract(&archive, &binary).unwrap();
        assert_eq!(fs::read(&binary).unwrap(), b"test");

        let mut duplicate = member(b'0');
        duplicate.extend(member(b'0'));
        duplicate.extend([0; 1024]);
        let mut symlink = member(b'2');
        symlink.extend([0; 1024]);
        for invalid in [valid[..514].to_vec(), duplicate, symlink, vec![0; 1024]] {
            fs::write(&archive, invalid).unwrap();
            assert!(extract(&archive, &binary).is_err());
        }
    }
}
