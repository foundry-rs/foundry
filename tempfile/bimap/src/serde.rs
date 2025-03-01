//! Implementations of `serde::Serialize` and `serde::Deserialize` for
//! `BiHashMap` and `BiBTreeMap`.
//!
//! You do not need to import anything from this module to use this
//! functionality, simply enable the `serde` feature in your dependency
//! manifest. Note that currently, this requires the `std` feature to also be
//! enabled, and thus cannot be used in `no_std` enviroments.
//!
//! # Examples
//!
//! You can easily serialize and deserialize bimaps with any serde-compatbile
//! serializer or deserializer.
//!
//! Serializing and deserializing a [`BiHashMap`]:
//!
//! ```
//! # use bimap::BiHashMap;
//! // create a new bimap
//! let mut map = BiHashMap::new();
//!
//! // insert some pairs
//! map.insert('A', 1);
//! map.insert('B', 2);
//! map.insert('C', 3);
//!
//! // convert the bimap to json
//! let json = serde_json::to_string(&map).unwrap();
//!
//! // convert the json back into a bimap
//! let map2 = serde_json::from_str(&json).unwrap();
//!
//! // check that the two bimaps are equal
//! assert_eq!(map, map2);
//! ```
//!
//! Serializing and deserializing a [`BiBTreeMap`]:
//! ```
//! # use bimap::BiBTreeMap;
//! // create a new bimap
//! let mut map = BiBTreeMap::new();
//!
//! // insert some pairs
//! map.insert('A', 3);
//! map.insert('B', 2);
//! map.insert('C', 1);
//!
//! // convert the bimap to json
//! let json = serde_json::to_string(&map).unwrap();
//!
//! // convert the json back into a bimap
//! let map2 = serde_json::from_str(&json).unwrap();
//!
//! // check that the two bimaps are equal
//! assert_eq!(map, map2);
//! ```
//!
//! Of course, this is only possible for bimaps where the values also implement
//! `Serialize` and `Deserialize` respectively:
//!
//! ```compile_fail
//! # use bimap::BiHashMap;
//! // this type doesn't implement Serialize or Deserialize!
//! #[derive(PartialEq, Eq, Hash)]
//! enum MyEnum { A, B, C }
//!
//! // create a bimap and add some pairs
//! let mut map = BiHashMap::new();
//! map.insert(MyEnum::A, 1);
//! map.insert(MyEnum::B, 2);
//! map.insert(MyEnum::C, 3);
//!
//! // this line will cause the code to fail to compile
//! let json = serde_json::to_string(&map).unwrap();
//! ```
//!
//! # Implementation details
//!
//! Bimaps are serialized and deserialized as a map data type in serde.
//! Consequentially, it is possible to serialize and deserialize bimaps to/from
//! other types that are represented the same way. *This is considered an
//! implementation detail and should not be relied upon.*
//!
//! For example, a bimap can be deserialized from the serialized form of a
//! standard [`HashMap`]. However, *deserializing a bimap silently overwrites
//! any conflicting pairs*, leading to non-deterministic results.
//! ```
//! # use std::collections::HashMap;
//! # use bimap::BiHashMap;
//! // construct a regular map
//! let mut map = HashMap::new();
//!
//! // insert some entries
//! // note that both 'B' and 'C' are associated with the value 2 here
//! map.insert('A', 1);
//! map.insert('B', 2);
//! map.insert('C', 2);
//!
//! // serialize the map
//! let json = serde_json::to_string(&map).unwrap();
//!
//! // deserialize it into a bimap
//! let bimap: BiHashMap<char, i32> = serde_json::from_str(&json).unwrap();
//!
//! // deserialization succeeds, but the bimap is now in a non-deterministic
//! // state - either ('B', 2) or ('C', 2) will have been overwritten while
//! // deserializing, but this depends on the iteration order of the original
//! // HashMap that was serialized.
//!
//! // we can still demonstrate that certain properties of the bimap are still
//! // in a known state, but this shouldn't be relied upon
//! assert_eq!(bimap.len(), 2);
//! assert_eq!(bimap.get_by_left(&'A'), Some(&1));
//! assert!(bimap.get_by_left(&'B') == Some(&2) || bimap.get_by_left(&'C') == Some(&2))
//! ```
//!
//! The reverse is also possible: bimaps may be serialized and then
//! deserialized as other compatible types, such as a [`HashMap`].
//!
//! ```
//! # use std::collections::HashMap;
//! # use bimap::BiHashMap;
//! // construct a bimap
//! let mut bimap = BiHashMap::new();
//!
//! // insert some pairs
//! bimap.insert('A', 1);
//! bimap.insert('B', 2);
//! bimap.insert('C', 3);
//!
//! // serialize the bimap
//! let json = serde_json::to_string(&bimap).unwrap();
//!
//! // deserialize it as a regular map
//! let map: HashMap<char, i32> = serde_json::from_str(&json).unwrap();
//!
//! // this succeeds and the result is sensible, but this is still an
//! // implementation detail and shouldn't be relied upon.
//! assert_eq!(map.len(), 3);
//! assert_eq!(map[&'A'], 1);
//! assert_eq!(map[&'B'], 2);
//! assert_eq!(map[&'C'], 3);
//! ```
//! [`BiHashMap`]: crate::BiHashMap
//! [`BiBTreeMap`]: crate::BiBTreeMap
//! [`HashMap`]: https://doc.rust-lang.org/std/collections/struct.HashMap.html

use crate::{BiBTreeMap, BiHashMap};
use serde::{
    de::{MapAccess, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};
use std::{
    default::Default,
    fmt::{Formatter, Result as FmtResult},
    hash::{BuildHasher, Hash},
    marker::PhantomData,
};

/// Serializer for `BiHashMap`
impl<L, R, LS, RS> Serialize for BiHashMap<L, R, LS, RS>
where
    L: Serialize + Eq + Hash,
    R: Serialize + Eq + Hash,
    LS: BuildHasher + Default,
    RS: BuildHasher + Default,
{
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.collect_map(self.iter())
    }
}

/// Visitor to construct `BiHashMap` from serialized map entries
struct BiHashMapVisitor<L, R, LS, RS> {
    marker: PhantomData<BiHashMap<L, R, LS, RS>>,
}

impl<'de, L, R, LS, RS> Visitor<'de> for BiHashMapVisitor<L, R, LS, RS>
where
    L: Deserialize<'de> + Eq + Hash,
    R: Deserialize<'de> + Eq + Hash,
    LS: BuildHasher + Default,
    RS: BuildHasher + Default,
{
    fn expecting(&self, f: &mut Formatter) -> FmtResult {
        write!(f, "a map")
    }

    type Value = BiHashMap<L, R, LS, RS>;
    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Self::Value, A::Error> {
        let mut map = match entries.size_hint() {
            Some(s) => BiHashMap::<L, R, LS, RS>::with_capacity_and_hashers(
                s,
                LS::default(),
                RS::default(),
            ),
            None => BiHashMap::<L, R, LS, RS>::with_hashers(LS::default(), RS::default()),
        };
        while let Some((l, r)) = entries.next_entry()? {
            map.insert(l, r);
        }
        Ok(map)
    }
}

/// Deserializer for `BiHashMap`
impl<'de, L, R, LS, RS> Deserialize<'de> for BiHashMap<L, R, LS, RS>
where
    L: Deserialize<'de> + Eq + Hash,
    R: Deserialize<'de> + Eq + Hash,
    LS: BuildHasher + Default,
    RS: BuildHasher + Default,
{
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        de.deserialize_map(BiHashMapVisitor::<L, R, LS, RS> {
            marker: PhantomData::default(),
        })
    }
}

/// Serializer for `BiBTreeMap`
impl<L, R> Serialize for BiBTreeMap<L, R>
where
    L: Serialize + Ord,
    R: Serialize + Ord,
{
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.collect_map(self.iter())
    }
}

/// Visitor to construct `BiBTreeMap` from serialized map entries
struct BiBTreeMapVisitor<L, R> {
    marker: PhantomData<BiBTreeMap<L, R>>,
}

impl<'de, L, R> Visitor<'de> for BiBTreeMapVisitor<L, R>
where
    L: Deserialize<'de> + Ord,
    R: Deserialize<'de> + Ord,
{
    fn expecting(&self, f: &mut Formatter) -> FmtResult {
        write!(f, "a map")
    }

    type Value = BiBTreeMap<L, R>;
    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Self::Value, A::Error> {
        let mut map = BiBTreeMap::new();
        while let Some((l, r)) = entries.next_entry()? {
            map.insert(l, r);
        }
        Ok(map)
    }
}

/// Deserializer for `BiBTreeMap`
impl<'de, L, R> Deserialize<'de> for BiBTreeMap<L, R>
where
    L: Deserialize<'de> + Ord,
    R: Deserialize<'de> + Ord,
{
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        de.deserialize_map(BiBTreeMapVisitor {
            marker: PhantomData::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::value::Error;
    use std::hash::BuildHasherDefault;

    #[test]
    fn serde_hash() {
        let mut bimap = BiHashMap::new();
        bimap.insert('a', 1);
        bimap.insert('b', 2);
        bimap.insert('c', 3);

        let json = serde_json::to_string(&bimap).unwrap();
        let bimap2 = serde_json::from_str(&json).unwrap();

        assert_eq!(bimap, bimap2);
    }

    #[test]
    fn serde_hash_w_fnv_hasher() {
        let hasher_builder = BuildHasherDefault::<fnv::FnvHasher>::default();
        let mut bimap = BiHashMap::<
            char,
            u8,
            BuildHasherDefault<fnv::FnvHasher>,
            BuildHasherDefault<fnv::FnvHasher>,
        >::with_capacity_and_hashers(
            4, hasher_builder.clone(), hasher_builder.clone()
        );
        bimap.insert('f', 1);
        bimap.insert('g', 2);
        bimap.insert('h', 3);

        let json = serde_json::to_string(&bimap).unwrap();
        let bimap2 = serde_json::from_str(&json).unwrap();

        assert_eq!(bimap, bimap2);
    }

    #[test]
    fn serde_hash_w_hashbrown_hasher() {
        let hasher_builder = hashbrown::hash_map::DefaultHashBuilder::default();
        let mut bimap = BiHashMap::<
            char,
            u8,
            hashbrown::hash_map::DefaultHashBuilder,
            hashbrown::hash_map::DefaultHashBuilder,
        >::with_capacity_and_hashers(
            4, hasher_builder.clone(), hasher_builder.clone()
        );
        bimap.insert('x', 1);
        bimap.insert('y', 2);
        bimap.insert('z', 3);

        let json = serde_json::to_string(&bimap).unwrap();
        let bimap2 = serde_json::from_str(&json).unwrap();

        assert_eq!(bimap, bimap2);
    }

    #[test]
    fn serde_btree() {
        let mut bimap = BiBTreeMap::new();
        bimap.insert('a', 1);
        bimap.insert('b', 2);
        bimap.insert('c', 3);

        let json = serde_json::to_string(&bimap).unwrap();
        let bimap2 = serde_json::from_str(&json).unwrap();

        assert_eq!(bimap, bimap2);
    }

    #[test]
    fn expecting_btree() {
        let visitor = BiBTreeMapVisitor {
            marker: PhantomData::<BiBTreeMap<char, i32>>,
        };
        let error_str = format!("{:?}", visitor.visit_bool::<Error>(true));
        let expected = "Err(Error(\"invalid type: boolean `true`, expected a map\"))";
        assert_eq!(error_str, expected);
    }

    #[test]
    fn expecting_hash() {
        let visitor = BiHashMapVisitor {
            marker: PhantomData::<BiHashMap<char, i32>>,
        };
        let error_str = format!("{:?}", visitor.visit_bool::<Error>(true));
        let expected = "Err(Error(\"invalid type: boolean `true`, expected a map\"))";
        assert_eq!(error_str, expected);
    }
}
