use super::*;
use std::string::String;

#[test]
fn it_works() {
    let mut map = IndexMap::new();
    assert_eq!(map.is_empty(), true);
    map.insert(1, ());
    map.insert(1, ());
    assert_eq!(map.len(), 1);
    assert!(map.get(&1).is_some());
    assert_eq!(map.is_empty(), false);
}

#[test]
fn new() {
    let map = IndexMap::<String, String>::new();
    println!("{:?}", map);
    assert_eq!(map.capacity(), 0);
    assert_eq!(map.len(), 0);
    assert_eq!(map.is_empty(), true);
}

#[test]
fn insert() {
    let insert = [0, 4, 2, 12, 8, 7, 11, 5];
    let not_present = [1, 3, 6, 9, 10];
    let mut map = IndexMap::with_capacity(insert.len());

    for (i, &elt) in insert.iter().enumerate() {
        assert_eq!(map.len(), i);
        map.insert(elt, elt);
        assert_eq!(map.len(), i + 1);
        assert_eq!(map.get(&elt), Some(&elt));
        assert_eq!(map[&elt], elt);
    }
    println!("{:?}", map);

    for &elt in &not_present {
        assert!(map.get(&elt).is_none());
    }
}

#[test]
fn insert_full() {
    let insert = vec![9, 2, 7, 1, 4, 6, 13];
    let present = vec![1, 6, 2];
    let mut map = IndexMap::with_capacity(insert.len());

    for (i, &elt) in insert.iter().enumerate() {
        assert_eq!(map.len(), i);
        let (index, existing) = map.insert_full(elt, elt);
        assert_eq!(existing, None);
        assert_eq!(Some(index), map.get_full(&elt).map(|x| x.0));
        assert_eq!(map.len(), i + 1);
    }

    let len = map.len();
    for &elt in &present {
        let (index, existing) = map.insert_full(elt, elt);
        assert_eq!(existing, Some(elt));
        assert_eq!(Some(index), map.get_full(&elt).map(|x| x.0));
        assert_eq!(map.len(), len);
    }
}

#[test]
fn insert_2() {
    let mut map = IndexMap::with_capacity(16);

    let mut keys = vec![];
    keys.extend(0..16);
    keys.extend(if cfg!(miri) { 32..64 } else { 128..267 });

    for &i in &keys {
        let old_map = map.clone();
        map.insert(i, ());
        for key in old_map.keys() {
            if map.get(key).is_none() {
                println!("old_map: {:?}", old_map);
                println!("map: {:?}", map);
                panic!("did not find {} in map", key);
            }
        }
    }

    for &i in &keys {
        assert!(map.get(&i).is_some(), "did not find {}", i);
    }
}

#[test]
fn insert_order() {
    let insert = [0, 4, 2, 12, 8, 7, 11, 5, 3, 17, 19, 22, 23];
    let mut map = IndexMap::new();

    for &elt in &insert {
        map.insert(elt, ());
    }

    assert_eq!(map.keys().count(), map.len());
    assert_eq!(map.keys().count(), insert.len());
    for (a, b) in insert.iter().zip(map.keys()) {
        assert_eq!(a, b);
    }
    for (i, k) in (0..insert.len()).zip(map.keys()) {
        assert_eq!(map.get_index(i).unwrap().0, k);
    }
}

#[test]
fn shift_insert() {
    let insert = [0, 4, 2, 12, 8, 7, 11, 5, 3, 17, 19, 22, 23];
    let mut map = IndexMap::new();

    for &elt in &insert {
        map.shift_insert(0, elt, ());
    }

    assert_eq!(map.keys().count(), map.len());
    assert_eq!(map.keys().count(), insert.len());
    for (a, b) in insert.iter().rev().zip(map.keys()) {
        assert_eq!(a, b);
    }
    for (i, k) in (0..insert.len()).zip(map.keys()) {
        assert_eq!(map.get_index(i).unwrap().0, k);
    }

    // "insert" that moves an existing entry
    map.shift_insert(0, insert[0], ());
    assert_eq!(map.keys().count(), insert.len());
    assert_eq!(insert[0], map.keys()[0]);
    for (a, b) in insert[1..].iter().rev().zip(map.keys().skip(1)) {
        assert_eq!(a, b);
    }
}

#[test]
fn insert_sorted_bad() {
    let mut map = IndexMap::new();
    map.insert(10, ());
    for i in 0..10 {
        map.insert(i, ());
    }

    // The binary search will want to insert this at the end (index == len()),
    // but that's only possible for *new* inserts. It should still be handled
    // without panicking though, and in this case it's simple enough that we
    // know the exact result. (But don't read this as an API guarantee!)
    assert_eq!(map.first(), Some((&10, &())));
    map.insert_sorted(10, ());
    assert_eq!(map.last(), Some((&10, &())));
    assert!(map.keys().copied().eq(0..=10));

    // Other out-of-order entries can also "insert" to a binary-searched
    // position, moving in either direction.
    map.move_index(5, 0);
    map.move_index(6, 10);
    assert_eq!(map.first(), Some((&5, &())));
    assert_eq!(map.last(), Some((&6, &())));
    map.insert_sorted(5, ()); // moves back up
    map.insert_sorted(6, ()); // moves back down
    assert!(map.keys().copied().eq(0..=10));
}

#[test]
fn grow() {
    let insert = [0, 4, 2, 12, 8, 7, 11];
    let not_present = [1, 3, 6, 9, 10];
    let mut map = IndexMap::with_capacity(insert.len());

    for (i, &elt) in insert.iter().enumerate() {
        assert_eq!(map.len(), i);
        map.insert(elt, elt);
        assert_eq!(map.len(), i + 1);
        assert_eq!(map.get(&elt), Some(&elt));
        assert_eq!(map[&elt], elt);
    }

    println!("{:?}", map);
    for &elt in &insert {
        map.insert(elt * 10, elt);
    }
    for &elt in &insert {
        map.insert(elt * 100, elt);
    }
    for (i, &elt) in insert.iter().cycle().enumerate().take(100) {
        map.insert(elt * 100 + i as i32, elt);
    }
    println!("{:?}", map);
    for &elt in &not_present {
        assert!(map.get(&elt).is_none());
    }
}

#[test]
fn reserve() {
    let mut map = IndexMap::<usize, usize>::new();
    assert_eq!(map.capacity(), 0);
    map.reserve(100);
    let capacity = map.capacity();
    assert!(capacity >= 100);
    for i in 0..capacity {
        assert_eq!(map.len(), i);
        map.insert(i, i * i);
        assert_eq!(map.len(), i + 1);
        assert_eq!(map.capacity(), capacity);
        assert_eq!(map.get(&i), Some(&(i * i)));
    }
    map.insert(capacity, std::usize::MAX);
    assert_eq!(map.len(), capacity + 1);
    assert!(map.capacity() > capacity);
    assert_eq!(map.get(&capacity), Some(&std::usize::MAX));
}

#[test]
fn try_reserve() {
    let mut map = IndexMap::<usize, usize>::new();
    assert_eq!(map.capacity(), 0);
    assert_eq!(map.try_reserve(100), Ok(()));
    assert!(map.capacity() >= 100);
    assert!(map.try_reserve(usize::MAX).is_err());
}

#[test]
fn shrink_to_fit() {
    let mut map = IndexMap::<usize, usize>::new();
    assert_eq!(map.capacity(), 0);
    for i in 0..100 {
        assert_eq!(map.len(), i);
        map.insert(i, i * i);
        assert_eq!(map.len(), i + 1);
        assert!(map.capacity() >= i + 1);
        assert_eq!(map.get(&i), Some(&(i * i)));
        map.shrink_to_fit();
        assert_eq!(map.len(), i + 1);
        assert_eq!(map.capacity(), i + 1);
        assert_eq!(map.get(&i), Some(&(i * i)));
    }
}

#[test]
fn remove() {
    let insert = [0, 4, 2, 12, 8, 7, 11, 5, 3, 17, 19, 22, 23];
    let mut map = IndexMap::new();

    for &elt in &insert {
        map.insert(elt, elt);
    }

    assert_eq!(map.keys().count(), map.len());
    assert_eq!(map.keys().count(), insert.len());
    for (a, b) in insert.iter().zip(map.keys()) {
        assert_eq!(a, b);
    }

    let remove_fail = [99, 77];
    let remove = [4, 12, 8, 7];

    for &key in &remove_fail {
        assert!(map.swap_remove_full(&key).is_none());
    }
    println!("{:?}", map);
    for &key in &remove {
        //println!("{:?}", map);
        let index = map.get_full(&key).unwrap().0;
        assert_eq!(map.swap_remove_full(&key), Some((index, key, key)));
    }
    println!("{:?}", map);

    for key in &insert {
        assert_eq!(map.get(key).is_some(), !remove.contains(key));
    }
    assert_eq!(map.len(), insert.len() - remove.len());
    assert_eq!(map.keys().count(), insert.len() - remove.len());
}

#[test]
fn remove_to_empty() {
    let mut map = indexmap! { 0 => 0, 4 => 4, 5 => 5 };
    map.swap_remove(&5).unwrap();
    map.swap_remove(&4).unwrap();
    map.swap_remove(&0).unwrap();
    assert!(map.is_empty());
}

#[test]
fn swap_remove_index() {
    let insert = [0, 4, 2, 12, 8, 7, 11, 5, 3, 17, 19, 22, 23];
    let mut map = IndexMap::new();

    for &elt in &insert {
        map.insert(elt, elt * 2);
    }

    let mut vector = insert.to_vec();
    let remove_sequence = &[3, 3, 10, 4, 5, 4, 3, 0, 1];

    // check that the same swap remove sequence on vec and map
    // have the same result.
    for &rm in remove_sequence {
        let out_vec = vector.swap_remove(rm);
        let (out_map, _) = map.swap_remove_index(rm).unwrap();
        assert_eq!(out_vec, out_map);
    }
    assert_eq!(vector.len(), map.len());
    for (a, b) in vector.iter().zip(map.keys()) {
        assert_eq!(a, b);
    }
}

#[test]
fn partial_eq_and_eq() {
    let mut map_a = IndexMap::new();
    map_a.insert(1, "1");
    map_a.insert(2, "2");
    let mut map_b = map_a.clone();
    assert_eq!(map_a, map_b);
    map_b.swap_remove(&1);
    assert_ne!(map_a, map_b);

    let map_c: IndexMap<_, String> = map_b.into_iter().map(|(k, v)| (k, v.into())).collect();
    assert_ne!(map_a, map_c);
    assert_ne!(map_c, map_a);
}

#[test]
fn extend() {
    let mut map = IndexMap::new();
    map.extend(vec![(&1, &2), (&3, &4)]);
    map.extend(vec![(5, 6)]);
    assert_eq!(
        map.into_iter().collect::<Vec<_>>(),
        vec![(1, 2), (3, 4), (5, 6)]
    );
}

#[test]
fn entry() {
    let mut map = IndexMap::new();

    map.insert(1, "1");
    map.insert(2, "2");
    {
        let e = map.entry(3);
        assert_eq!(e.index(), 2);
        let e = e.or_insert("3");
        assert_eq!(e, &"3");
    }

    let e = map.entry(2);
    assert_eq!(e.index(), 1);
    assert_eq!(e.key(), &2);
    match e {
        Entry::Occupied(ref e) => assert_eq!(e.get(), &"2"),
        Entry::Vacant(_) => panic!(),
    }
    assert_eq!(e.or_insert("4"), &"2");
}

#[test]
fn entry_and_modify() {
    let mut map = IndexMap::new();

    map.insert(1, "1");
    map.entry(1).and_modify(|x| *x = "2");
    assert_eq!(Some(&"2"), map.get(&1));

    map.entry(2).and_modify(|x| *x = "doesn't exist");
    assert_eq!(None, map.get(&2));
}

#[test]
fn entry_or_default() {
    let mut map = IndexMap::new();

    #[derive(Debug, PartialEq)]
    enum TestEnum {
        DefaultValue,
        NonDefaultValue,
    }

    impl Default for TestEnum {
        fn default() -> Self {
            TestEnum::DefaultValue
        }
    }

    map.insert(1, TestEnum::NonDefaultValue);
    assert_eq!(&mut TestEnum::NonDefaultValue, map.entry(1).or_default());

    assert_eq!(&mut TestEnum::DefaultValue, map.entry(2).or_default());
}

#[test]
fn occupied_entry_key() {
    // These keys match hash and equality, but their addresses are distinct.
    let (k1, k2) = (&mut 1, &mut 1);
    let k1_ptr = k1 as *const i32;
    let k2_ptr = k2 as *const i32;
    assert_ne!(k1_ptr, k2_ptr);

    let mut map = IndexMap::new();
    map.insert(k1, "value");
    match map.entry(k2) {
        Entry::Occupied(ref e) => {
            // `OccupiedEntry::key` should reference the key in the map,
            // not the key that was used to find the entry.
            let ptr = *e.key() as *const i32;
            assert_eq!(ptr, k1_ptr);
            assert_ne!(ptr, k2_ptr);
        }
        Entry::Vacant(_) => panic!(),
    }
}

#[test]
fn get_index_entry() {
    let mut map = IndexMap::new();

    assert!(map.get_index_entry(0).is_none());
    assert!(map.first_entry().is_none());
    assert!(map.last_entry().is_none());

    map.insert(0, "0");
    map.insert(1, "1");
    map.insert(2, "2");
    map.insert(3, "3");

    assert!(map.get_index_entry(4).is_none());

    {
        let e = map.get_index_entry(1).unwrap();
        assert_eq!(*e.key(), 1);
        assert_eq!(*e.get(), "1");
        assert_eq!(e.swap_remove(), "1");
    }

    {
        let mut e = map.get_index_entry(1).unwrap();
        assert_eq!(*e.key(), 3);
        assert_eq!(*e.get(), "3");
        assert_eq!(e.insert("4"), "3");
    }

    assert_eq!(*map.get(&3).unwrap(), "4");

    {
        let e = map.first_entry().unwrap();
        assert_eq!(*e.key(), 0);
        assert_eq!(*e.get(), "0");
    }

    {
        let e = map.last_entry().unwrap();
        assert_eq!(*e.key(), 2);
        assert_eq!(*e.get(), "2");
    }
}

#[test]
fn from_entries() {
    let mut map = IndexMap::from([(1, "1"), (2, "2"), (3, "3")]);

    {
        let e = match map.entry(1) {
            Entry::Occupied(e) => IndexedEntry::from(e),
            Entry::Vacant(_) => panic!(),
        };
        assert_eq!(e.index(), 0);
        assert_eq!(*e.key(), 1);
        assert_eq!(*e.get(), "1");
    }

    {
        let e = match map.get_index_entry(1) {
            Some(e) => OccupiedEntry::from(e),
            None => panic!(),
        };
        assert_eq!(e.index(), 1);
        assert_eq!(*e.key(), 2);
        assert_eq!(*e.get(), "2");
    }
}

#[test]
fn keys() {
    let vec = vec![(1, 'a'), (2, 'b'), (3, 'c')];
    let map: IndexMap<_, _> = vec.into_iter().collect();
    let keys: Vec<_> = map.keys().copied().collect();
    assert_eq!(keys.len(), 3);
    assert!(keys.contains(&1));
    assert!(keys.contains(&2));
    assert!(keys.contains(&3));
}

#[test]
fn into_keys() {
    let vec = vec![(1, 'a'), (2, 'b'), (3, 'c')];
    let map: IndexMap<_, _> = vec.into_iter().collect();
    let keys: Vec<i32> = map.into_keys().collect();
    assert_eq!(keys.len(), 3);
    assert!(keys.contains(&1));
    assert!(keys.contains(&2));
    assert!(keys.contains(&3));
}

#[test]
fn values() {
    let vec = vec![(1, 'a'), (2, 'b'), (3, 'c')];
    let map: IndexMap<_, _> = vec.into_iter().collect();
    let values: Vec<_> = map.values().copied().collect();
    assert_eq!(values.len(), 3);
    assert!(values.contains(&'a'));
    assert!(values.contains(&'b'));
    assert!(values.contains(&'c'));
}

#[test]
fn values_mut() {
    let vec = vec![(1, 1), (2, 2), (3, 3)];
    let mut map: IndexMap<_, _> = vec.into_iter().collect();
    for value in map.values_mut() {
        *value *= 2
    }
    let values: Vec<_> = map.values().copied().collect();
    assert_eq!(values.len(), 3);
    assert!(values.contains(&2));
    assert!(values.contains(&4));
    assert!(values.contains(&6));
}

#[test]
fn into_values() {
    let vec = vec![(1, 'a'), (2, 'b'), (3, 'c')];
    let map: IndexMap<_, _> = vec.into_iter().collect();
    let values: Vec<char> = map.into_values().collect();
    assert_eq!(values.len(), 3);
    assert!(values.contains(&'a'));
    assert!(values.contains(&'b'));
    assert!(values.contains(&'c'));
}

#[test]
fn drain_range() {
    // Test the various heuristics of `erase_indices`
    for range in [
        0..0,   // nothing erased
        10..90, // reinsert the few kept (..10 and 90..)
        80..90, // update the few to adjust (80..)
        20..30, // sweep everything
    ] {
        let mut vec = Vec::from_iter(0..100);
        let mut map: IndexMap<i32, ()> = (0..100).map(|i| (i, ())).collect();
        drop(vec.drain(range.clone()));
        drop(map.drain(range));
        assert!(vec.iter().eq(map.keys()));
        for (i, x) in vec.iter().enumerate() {
            assert_eq!(map.get_index_of(x), Some(i));
        }
    }
}

#[test]
#[cfg(feature = "std")]
fn from_array() {
    let map = IndexMap::from([(1, 2), (3, 4)]);
    let mut expected = IndexMap::new();
    expected.insert(1, 2);
    expected.insert(3, 4);

    assert_eq!(map, expected)
}

#[test]
fn iter_default() {
    struct K;
    struct V;
    fn assert_default<T>()
    where
        T: Default + Iterator,
    {
        assert!(T::default().next().is_none());
    }
    assert_default::<Iter<'static, K, V>>();
    assert_default::<IterMut<'static, K, V>>();
    assert_default::<IterMut2<'static, K, V>>();
    assert_default::<IntoIter<K, V>>();
    assert_default::<Keys<'static, K, V>>();
    assert_default::<IntoKeys<K, V>>();
    assert_default::<Values<'static, K, V>>();
    assert_default::<ValuesMut<'static, K, V>>();
    assert_default::<IntoValues<K, V>>();
}

#[test]
fn test_binary_search_by() {
    // adapted from std's test for binary_search
    let b: IndexMap<_, i32> = []
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&5)), Err(0));

    let b: IndexMap<_, i32> = [4]
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&3)), Err(0));
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&4)), Ok(0));
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&5)), Err(1));

    let b: IndexMap<_, i32> = [1, 2, 4, 6, 8, 9]
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&5)), Err(3));
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&6)), Ok(3));
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&7)), Err(4));
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&8)), Ok(4));

    let b: IndexMap<_, i32> = [1, 2, 4, 5, 6, 8]
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&9)), Err(6));

    let b: IndexMap<_, i32> = [1, 2, 4, 6, 7, 8, 9]
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&6)), Ok(3));
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&5)), Err(3));
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&8)), Ok(5));

    let b: IndexMap<_, i32> = [1, 2, 4, 5, 6, 8, 9]
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&7)), Err(5));
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&0)), Err(0));

    let b: IndexMap<_, i32> = [1, 3, 3, 3, 7]
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&0)), Err(0));
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&1)), Ok(0));
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&2)), Err(1));
    assert!(match b.binary_search_by(|_, x| x.cmp(&3)) {
        Ok(1..=3) => true,
        _ => false,
    });
    assert!(match b.binary_search_by(|_, x| x.cmp(&3)) {
        Ok(1..=3) => true,
        _ => false,
    });
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&4)), Err(4));
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&5)), Err(4));
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&6)), Err(4));
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&7)), Ok(4));
    assert_eq!(b.binary_search_by(|_, x| x.cmp(&8)), Err(5));
}

#[test]
fn test_binary_search_by_key() {
    // adapted from std's test for binary_search
    let b: IndexMap<_, i32> = []
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.binary_search_by_key(&5, |_, &x| x), Err(0));

    let b: IndexMap<_, i32> = [4]
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.binary_search_by_key(&3, |_, &x| x), Err(0));
    assert_eq!(b.binary_search_by_key(&4, |_, &x| x), Ok(0));
    assert_eq!(b.binary_search_by_key(&5, |_, &x| x), Err(1));

    let b: IndexMap<_, i32> = [1, 2, 4, 6, 8, 9]
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.binary_search_by_key(&5, |_, &x| x), Err(3));
    assert_eq!(b.binary_search_by_key(&6, |_, &x| x), Ok(3));
    assert_eq!(b.binary_search_by_key(&7, |_, &x| x), Err(4));
    assert_eq!(b.binary_search_by_key(&8, |_, &x| x), Ok(4));

    let b: IndexMap<_, i32> = [1, 2, 4, 5, 6, 8]
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.binary_search_by_key(&9, |_, &x| x), Err(6));

    let b: IndexMap<_, i32> = [1, 2, 4, 6, 7, 8, 9]
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.binary_search_by_key(&6, |_, &x| x), Ok(3));
    assert_eq!(b.binary_search_by_key(&5, |_, &x| x), Err(3));
    assert_eq!(b.binary_search_by_key(&8, |_, &x| x), Ok(5));

    let b: IndexMap<_, i32> = [1, 2, 4, 5, 6, 8, 9]
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.binary_search_by_key(&7, |_, &x| x), Err(5));
    assert_eq!(b.binary_search_by_key(&0, |_, &x| x), Err(0));

    let b: IndexMap<_, i32> = [1, 3, 3, 3, 7]
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.binary_search_by_key(&0, |_, &x| x), Err(0));
    assert_eq!(b.binary_search_by_key(&1, |_, &x| x), Ok(0));
    assert_eq!(b.binary_search_by_key(&2, |_, &x| x), Err(1));
    assert!(match b.binary_search_by_key(&3, |_, &x| x) {
        Ok(1..=3) => true,
        _ => false,
    });
    assert!(match b.binary_search_by_key(&3, |_, &x| x) {
        Ok(1..=3) => true,
        _ => false,
    });
    assert_eq!(b.binary_search_by_key(&4, |_, &x| x), Err(4));
    assert_eq!(b.binary_search_by_key(&5, |_, &x| x), Err(4));
    assert_eq!(b.binary_search_by_key(&6, |_, &x| x), Err(4));
    assert_eq!(b.binary_search_by_key(&7, |_, &x| x), Ok(4));
    assert_eq!(b.binary_search_by_key(&8, |_, &x| x), Err(5));
}

#[test]
fn test_partition_point() {
    // adapted from std's test for partition_point
    let b: IndexMap<_, i32> = []
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.partition_point(|_, &x| x < 5), 0);

    let b: IndexMap<_, i32> = [4]
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.partition_point(|_, &x| x < 3), 0);
    assert_eq!(b.partition_point(|_, &x| x < 4), 0);
    assert_eq!(b.partition_point(|_, &x| x < 5), 1);

    let b: IndexMap<_, i32> = [1, 2, 4, 6, 8, 9]
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.partition_point(|_, &x| x < 5), 3);
    assert_eq!(b.partition_point(|_, &x| x < 6), 3);
    assert_eq!(b.partition_point(|_, &x| x < 7), 4);
    assert_eq!(b.partition_point(|_, &x| x < 8), 4);

    let b: IndexMap<_, i32> = [1, 2, 4, 5, 6, 8]
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.partition_point(|_, &x| x < 9), 6);

    let b: IndexMap<_, i32> = [1, 2, 4, 6, 7, 8, 9]
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.partition_point(|_, &x| x < 6), 3);
    assert_eq!(b.partition_point(|_, &x| x < 5), 3);
    assert_eq!(b.partition_point(|_, &x| x < 8), 5);

    let b: IndexMap<_, i32> = [1, 2, 4, 5, 6, 8, 9]
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.partition_point(|_, &x| x < 7), 5);
    assert_eq!(b.partition_point(|_, &x| x < 0), 0);

    let b: IndexMap<_, i32> = [1, 3, 3, 3, 7]
        .into_iter()
        .enumerate()
        .map(|(i, x)| (i + 100, x))
        .collect();
    assert_eq!(b.partition_point(|_, &x| x < 0), 0);
    assert_eq!(b.partition_point(|_, &x| x < 1), 0);
    assert_eq!(b.partition_point(|_, &x| x < 2), 1);
    assert_eq!(b.partition_point(|_, &x| x < 3), 1);
    assert_eq!(b.partition_point(|_, &x| x < 4), 4);
    assert_eq!(b.partition_point(|_, &x| x < 5), 4);
    assert_eq!(b.partition_point(|_, &x| x < 6), 4);
    assert_eq!(b.partition_point(|_, &x| x < 7), 4);
    assert_eq!(b.partition_point(|_, &x| x < 8), 5);
}

macro_rules! move_index_oob {
    ($test:ident, $from:expr, $to:expr) => {
        #[test]
        #[should_panic(expected = "index out of bounds")]
        fn $test() {
            let mut map: IndexMap<i32, ()> = (0..10).map(|k| (k, ())).collect();
            map.move_index($from, $to);
        }
    };
}
move_index_oob!(test_move_index_out_of_bounds_0_10, 0, 10);
move_index_oob!(test_move_index_out_of_bounds_0_max, 0, usize::MAX);
move_index_oob!(test_move_index_out_of_bounds_10_0, 10, 0);
move_index_oob!(test_move_index_out_of_bounds_max_0, usize::MAX, 0);
