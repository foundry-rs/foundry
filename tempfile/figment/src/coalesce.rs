use crate::Profile;
use crate::value::{Value, Map};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Order {
    Merge,
    Join,
    Adjoin,
    Admerge,
}

pub trait Coalescible: Sized {
    fn coalesce(self, other: Self, order: Order) -> Self;
    fn merge(self, other: Self) -> Self { self.coalesce(other, Order::Merge) }
}

impl Coalescible for Profile {
    fn coalesce(self, other: Self, order: Order) -> Self {
        match order {
            Order::Join | Order::Adjoin => self,
            Order::Merge | Order::Admerge => other,
        }
    }
}

impl Coalescible for Value {
    fn coalesce(self, other: Self, o: Order) -> Self {
        use {Value::Dict as D, Value::Array as A, Order::*};
        match (self, other, o) {
            (D(t, a), D(_, b), Join | Adjoin) | (D(_, a), D(t, b), Merge | Admerge) => D(t, a.coalesce(b, o)),
            (A(t, mut a), A(_, b), Adjoin | Admerge) => A(t, { a.extend(b); a }),
            (v, _, Join | Adjoin) | (_, v, Merge | Admerge) => v,
        }
    }
}

impl<K: Eq + std::hash::Hash + Ord, V: Coalescible> Coalescible for Map<K, V> {
    fn coalesce(self, mut other: Self, order: Order) -> Self {
        let mut joined = Map::new();
        for (a_key, a_val) in self {
            match other.remove(&a_key) {
                Some(b_val) => joined.insert(a_key, a_val.coalesce(b_val, order)),
                None => joined.insert(a_key, a_val),
            };
        }

        // `b` contains `b - a`, i.e, additions. keep them all.
        joined.extend(other);
        joined
    }
}
