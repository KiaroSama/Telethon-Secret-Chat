#![allow(clippy::unwrap_used)]
use std::path::{Path, PathBuf};
pub fn walk(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    files.sort();
    files
}
