//! Validate that each legacy identity has exactly one owner mapping.

use std::collections::BTreeMap;

pub fn check_partitions(
    aggregate: &BTreeMap<String, String>,
    partitions: &[BTreeMap<String, String>],
) -> Result<(), &'static str> {
    let mut union = BTreeMap::new();
    for partition in partitions {
        for (case, target) in partition {
            if union.insert(case.clone(), target.clone()).is_some() {
                return Err("A legacy identity appears in multiple owner ledgers");
            }
        }
    }
    if &union != aggregate {
        return Err("Owner ledgers disagree with the aggregate migration mapping");
    }
    Ok(())
}
