//! Reject ambiguous persisted identities before any snapshot can be rewritten.

use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    fmt,
    marker::PhantomData,
};

use serde::{Deserialize, Deserializer, de};

pub(crate) fn deserialize_unique_map<'de, D, K, V>(
    deserializer: D,
) -> Result<BTreeMap<K, V>, D::Error>
where
    D: Deserializer<'de>,
    K: Deserialize<'de> + Ord,
    V: Deserialize<'de>,
{
    struct UniqueMap<K, V>(PhantomData<(K, V)>);

    impl<'de, K, V> de::Visitor<'de> for UniqueMap<K, V>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
    {
        type Value = BTreeMap<K, V>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a map with unique persisted identities")
        }

        fn visit_map<A: de::MapAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
            let mut entries = BTreeMap::new();
            while let Some(key) = input.next_key()? {
                match entries.entry(key) {
                    Entry::Vacant(entry) => {
                        entry.insert(input.next_value()?);
                    }
                    Entry::Occupied(_) => {
                        return Err(de::Error::custom("duplicate persisted map identity"));
                    }
                }
            }
            Ok(entries)
        }
    }

    deserializer.deserialize_map(UniqueMap(PhantomData))
}

pub(crate) fn deserialize_unique_set<'de, D, T>(deserializer: D) -> Result<BTreeSet<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Ord,
{
    struct UniqueSet<T>(PhantomData<T>);

    impl<'de, T: Deserialize<'de> + Ord> de::Visitor<'de> for UniqueSet<T> {
        type Value = BTreeSet<T>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a set of unique persisted identities")
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
            let mut entries = BTreeSet::new();
            while let Some(value) = input.next_element()? {
                if !entries.insert(value) {
                    return Err(de::Error::custom("duplicate persisted set identity"));
                }
            }
            Ok(entries)
        }
    }

    deserializer.deserialize_seq(UniqueSet(PhantomData))
}

pub(crate) fn unique_map_from_slice<'de, K, V>(
    bytes: &'de [u8],
) -> serde_json::Result<BTreeMap<K, V>>
where
    K: Deserialize<'de> + Ord,
    V: Deserialize<'de>,
{
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let entries = deserialize_unique_map(&mut decoder)?;
    decoder.end()?;
    Ok(entries)
}
