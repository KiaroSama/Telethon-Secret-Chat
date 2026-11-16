//! MIME aliases and first-glob lookups from Qt5.15.19 Apache Tika.
#[path = "mime_aliases.rs"]
mod aliases;
#[path = "mime_globs_0.rs"]
mod first;
#[path = "mime_globs_1.rs"]
mod second;
pub use aliases::alias;
pub fn first_glob(mime: &str) -> &'static str {
    first::lookup(mime)
        .or_else(|| second::lookup(mime))
        .unwrap_or("")
}
