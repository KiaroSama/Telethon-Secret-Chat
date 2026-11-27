//! Qt MIME glob tables and the frozen Desktop export lookup order.

use std::{collections::HashMap, sync::LazyLock};

// Ordered data is extracted from baseline git blobs, independently of oracle outputs.
type Table = Vec<[&'static str; 2]>;
type Lookup = Result<HashMap<&'static str, &'static str>, String>;
fn table(data: &'static str) -> Result<Table, String> {
    serde_json::from_str(data).map_err(|e| format!("ValueError: invalid embedded MIME table: {e}"))
}
fn lookup(data: &'static str) -> Lookup {
    Ok(table(data)?.into_iter().map(|[k, v]| (k, v)).collect())
}
const FIRST_GLOBS: &str = include_str!("../../data/first_globs.json");
const ALIASES: &str = include_str!("../../data/aliases.json");
static FIRST: LazyLock<Lookup> = LazyLock::new(|| lookup(FIRST_GLOBS));
static ALIAS: LazyLock<Lookup> = LazyLock::new(|| lookup(ALIASES));
static KNOWN: LazyLock<Lookup> =
    LazyLock::new(|| lookup(include_str!("../../data/known_globs.json")));

fn ordered(data: &'static str) -> Result<Vec<Vec<String>>, String> {
    Ok(table(data)?
        .into_iter()
        .map(|pair| pair.map(str::to_owned).into())
        .collect())
}
pub fn ordered_first_globs() -> Result<Vec<Vec<String>>, String> {
    ordered(FIRST_GLOBS)
}
pub fn ordered_aliases() -> Result<Vec<Vec<String>>, String> {
    ordered(ALIASES)
}
pub fn mime_first_glob(mime: String) -> Result<String, String> {
    let known = KNOWN.as_ref().map_err(Clone::clone)?;
    if let Some(glob) = known.get(mime.as_str()) {
        return Ok((*glob).to_owned());
    }
    let mime = if mime == "audio/mpeg3" {
        "audio/mp3"
    } else {
        &mime
    };
    let alias = ALIAS.as_ref().map_err(Clone::clone)?;
    let key = alias.get(mime).copied().unwrap_or(mime);
    Ok(FIRST
        .as_ref()
        .map_err(Clone::clone)?
        .get(key)
        .copied()
        .unwrap_or_default()
        .to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_and_consumer_overrides_match_the_baseline() {
        assert_eq!(mime_first_glob("audio/mpeg3".into()), Ok("*.mpga".into()));
        assert_eq!(
            mime_first_glob("application/x-tgsticker".into()),
            Ok("*.tgs".into())
        );
        assert_eq!(mime_first_glob("IMAGE/JPEG".into()), Ok(String::new()));
    }

    #[test]
    fn both_source_tables_retain_their_original_order() {
        let first = ordered_first_globs();
        let aliases = ordered_aliases();
        assert_eq!(
            first.map(|values| (values.len(), values[0][0].clone())),
            Ok((925, "application/andrew-inset".into()))
        );
        assert_eq!(
            aliases.map(|values| (values.len(), values[0][0].clone())),
            Ok((150, "application/acad".into()))
        );
    }
}
