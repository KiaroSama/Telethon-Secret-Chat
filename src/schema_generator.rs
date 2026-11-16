//! Deterministic native secret-schema generation with atomic publication and read-only checking.
use crate::error::{Error, Result};
use grammers_tl_gen::{Config, Outputs, generate_rust_code};
use grammers_tl_parser::parse_tl_file;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};
#[derive(Debug, Clone)]
pub struct Generator {
    pub formatter: PathBuf,
}
impl Default for Generator {
    fn default() -> Self {
        Self {
            formatter: std::env::var_os("RUSTFMT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("rustfmt")),
        }
    }
}
fn normalize(source: &str) -> Result<String> {
    let mut counts = BTreeMap::new();
    for line in source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
    {
        if let Some((name, _)) = line.split_once('#') {
            *counts.entry(name.to_owned()).or_insert(0usize) += 1;
        }
    }
    let mut result = String::new();
    for line in source.lines() {
        if !line.trim_start().starts_with("//")
            && let Some((name, rest)) = line.split_once('#')
            && let Some(count) = counts.get_mut(name)
        {
            *count -= 1;
            if *count > 0 {
                let id = rest.split_whitespace().next().ok_or(Error::InvalidState)?;
                result.push_str(&format!("{name}_legacy{id}#{rest}\n"));
                continue;
            }
        }
        result.push_str(line);
        result.push('\n');
    }
    Ok(result)
}
/// Produce a single module containing all four official generator outputs. Formatting is separate.
pub fn render(source: &str) -> Result<Vec<u8>> {
    let normalized = normalize(source)?;
    let definitions = parse_tl_file(&normalized)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::InvalidState)?;
    let mut outputs = Outputs {
        common: Vec::new(),
        types: Vec::new(),
        functions: Vec::new(),
        enums: Vec::new(),
    };
    generate_rust_code(
        &mut outputs,
        &definitions,
        144,
        &Config {
            gen_name_for_id: true,
            impl_debug: false,
            ..Default::default()
        },
    )
    .map_err(|_| Error::Storage)?;
    outputs.enums = stable_getters(&outputs.enums)?;
    let mut result = b"// Generated Rust secret-chat schema. Do not edit by hand.\n".to_vec();
    result.extend(outputs.common);
    for (name, data) in [
        ("types", outputs.types),
        ("functions", outputs.functions),
        ("enums", outputs.enums),
    ] {
        result.extend_from_slice(format!("\npub mod {name} {{\n").as_bytes());
        result.extend(data);
        result.extend_from_slice(b"\n}\n");
    }
    Ok(result)
}

// The pinned upstream generator intersects common fields with a HashSet.
// Preserve every getter and body, but order inherent methods by their names
// before formatting so independently generated modules have identical bytes.
fn stable_getters(source: &[u8]) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(source).map_err(|_| Error::InvalidState)?;
    let mut file = syn::parse_file(source).map_err(|_| Error::InvalidState)?;
    for item in &mut file.items {
        if let syn::Item::Impl(implementation) = item
            && implementation.trait_.is_none()
            && implementation
                .items
                .iter()
                .all(|item| matches!(item, syn::ImplItem::Fn(_)))
        {
            implementation.items.sort_by_cached_key(|item| match item {
                syn::ImplItem::Fn(method) => method.sig.ident.to_string(),
                _ => String::new(),
            });
        }
    }
    Ok(quote::quote!(#file).to_string().into_bytes())
}
impl Generator {
    /// A true result means generation succeeded, or an existing file matched in check mode.
    /// Check mode never creates an absent output and never modifies the published file.
    pub fn generate(&self, schema: &Path, target: &Path, check: bool) -> Result<bool> {
        if check && !target.is_file() {
            return Ok(false);
        }
        let source = fs::read_to_string(schema).map_err(|_| Error::Storage)?;
        let bytes = render(&source)?;
        let target = std::path::absolute(target).map_err(|_| Error::Storage)?;
        let parent = target.parent().ok_or(Error::Storage)?;
        if !parent.exists() {
            if check {
                return Ok(false);
            }
            fs::create_dir_all(parent).map_err(|_| Error::Storage)?;
        }
        let mut staging = tempfile::Builder::new()
            .prefix(".secret-schema-")
            .suffix(".rs")
            .tempfile_in(parent)
            .map_err(|_| Error::Storage)?;
        staging.write_all(&bytes).map_err(|_| Error::Storage)?;
        staging.flush().map_err(|_| Error::Storage)?;
        let status = Command::new(&self.formatter)
            .arg("--edition")
            .arg("2024")
            .arg(staging.path())
            .status()
            .map_err(|_| Error::Storage)?;
        if !status.success() {
            return Err(Error::Storage);
        }
        let formatted = fs::read(staging.path()).map_err(|_| Error::Storage)?;
        if check {
            return fs::read(&target)
                .map(|old| old == formatted)
                .map_err(|_| Error::Storage);
        }
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(staging.path())
            .and_then(|file| file.sync_all())
            .map_err(|_| Error::Storage)?;
        staging.persist(&target).map_err(|_| Error::Storage)?;
        #[cfg(unix)]
        fs::File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|_| Error::Storage)?;
        Ok(true)
    }
}
