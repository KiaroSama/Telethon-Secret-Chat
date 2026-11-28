//! Bounded, atomic completion of links whose target page was unknown while streaming.
use super::{
    error::{ExportError, Result},
    html_writer::messages_file,
};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::Path,
};

pub(super) fn resolve(path: &Path, targets: &BTreeMap<i64, usize>) -> Result<()> {
    let result = (|| -> std::io::Result<()> {
        let source = File::open(path)?;
        let parent = path
            .parent()
            .ok_or_else(|| std::io::Error::other("missing output parent"))?;
        let mut output = crate::temporary::create(parent, ".export-links-")
            .map_err(|_| std::io::Error::other("output staging failed"))?;
        {
            let mut source = BufReader::new(source);
            let mut destination = BufWriter::new(&mut *output);
            let mut pending = Vec::with_capacity(8352);
            let mut block = [0; 8192];
            const PREFIX: &[u8] = b"<a href=\"#go_to_message";
            loop {
                let count = source.read(&mut block)?;
                pending.extend_from_slice(&block[..count]);
                // A complete generated anchor is shorter than this even for i64::MIN.
                let boundary = if count == 0 {
                    pending.len()
                } else {
                    pending.len().saturating_sub(160)
                };
                let (mut at, mut plain) = (0, 0);
                while at < boundary {
                    if pending[at..].starts_with(PREFIX) {
                        let digits = &pending[at + PREFIX.len()..];
                        if let Some(end) = digits.iter().position(|byte| *byte == b'\"')
                            && let Ok(id) = std::str::from_utf8(&digits[..end])
                                .unwrap_or("")
                                .parse::<i64>()
                            && let Some(page) = targets.get(&id)
                        {
                            let needle = format!(
                                "<a href=\"#go_to_message{id}\" onclick=\"return GoToMessage({id})\">"
                            );
                            if pending[at..].starts_with(needle.as_bytes()) {
                                destination.write_all(&pending[plain..at])?;
                                write!(
                                    destination,
                                    "<a href=\"{}#go_to_message{id}\">",
                                    messages_file(*page)
                                )?;
                                at += needle.len();
                                plain = at;
                                continue;
                            }
                        }
                    }
                    at += 1;
                }
                destination.write_all(&pending[plain..at])?;
                pending.drain(..at);
                if count == 0 {
                    break;
                }
            }
            destination.flush()?;
        }
        output.sync_all()?;
        output
            .persist(path)
            .map_err(|_| std::io::Error::other("output replacement failed"))?;
        Ok(())
    })();
    result.map_err(|_| ExportError::Filesystem)
}
