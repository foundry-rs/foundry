use std::fmt;
use std::result;
use std::cell::Cell;
use std::marker::PhantomData;
use std::borrow::Cow;

use serde::Deserialize;
use serde::de::{self, Deserializer, IntoDeserializer, Visitor};
use serde::de::{SeqAccess, MapAccess, VariantAccess};

use crate::Figment;
use crate::error::{Error, Kind, Result};
use crate::value::{Value, Num, Empty, Dict, Tag};

pub trait Interpreter {
    fn interpret_as_bool(v: &Value) -> Cow<'_, Value> {
        Cow::Borrowed(v)
    }

    fn interpret_as_num(v: &Value) -> Cow<'_, Value> {
        Cow::Borrowed(v)
    }
}

pub struct DefaultInterpreter;
impl Interpreter for DefaultInterpreter { }

pub struct LossyInterpreter;
impl Interpreter for LossyInterpreter {
    fn interpret_as_bool(v: &Value) -> Cow<'_, Value> {
        v.to_bool_lossy()
            .map(|b| Cow::Owned(Value::Bool(v.tag(), b)))
            .unwrap_or(Cow::Borrowed(v))
    }

    fn interpret_as_num(v: &Value) -> Cow<'_, Value> {
        v.to_num_lossy()
            .map(|n| Cow::Owned(Value::Num(v.tag(), n)))
            .unwrap_or(Cow::Borrowed(v))
    }
}

pub struct ConfiguredValueDe<'c, I = DefaultInterpreter> {
    pub config: &'c Figment,
    pub value: &'c Value,
    pub readable: Cell<bool>,
    _phantom: PhantomData<I>
}

impl<'c, I: Interpreter> ConfiguredValueDe<'c, I> {
    pub fn from(config: &'c Figment, value: &'c Value) -> Self {
        Self { config, value, readable: Cell::from(true), _phantom: PhantomData }
    }
}

/// Like [`serde::forward_to_deserialize_any`] but applies `$apply` to
/// `&self` first, then calls `deserialize_any()` on the returned value, and
/// finally maps any error produced using `$errmap`:
///   - $apply(&self).deserialize_any(visitor).map_err($errmap)
macro_rules! apply_then_forward_to_deserialize_any {
    ( $( $($f:ident),+ => |$this:pat| $apply:expr, $errmap:expr),* $(,)? ) => {
        $(
            $(
                fn $f<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
                    let $this = &self;
                    $apply.deserialize_any(visitor).map_err($errmap)
                }
            )+
        )*
    }
}

impl<'de: 'c, 'c, I: Interpreter> Deserializer<'de> for ConfiguredValueDe<'c, I> {
    type Error = Error;

    fn deserialize_any<V>(self, v: V) -> Result<V::Value>
        where V: de::Visitor<'de>
    {
        let maker = |v| Self::from(self.config, v);
        let result = match *self.value {
            Value::String(_, ref s) => v.visit_str(s),
            Value::Char(_, c) => v.visit_char(c),
            Value::Bool(_, b) => v.visit_bool(b),
            Value::Num(_, n) => n.deserialize_any(v),
            Value::Empty(_, e) => e.deserialize_any(v),
            Value::Dict(_, ref map) => v.visit_map(MapDe::new(map, maker)),
            Value::Array(_, ref seq) => v.visit_seq(SeqDe::new(seq, maker)),
        };

        result.map_err(|e| e.retagged(self.value.tag()).resolved(self.config))
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value>
        where V: Visitor<'de>
    {
        let (config, tag) = (self.config, self.value.tag());
        let result = match self.value {
            Value::Empty(_, val) => val.deserialize_any(visitor),
            _ => visitor.visit_some(self)
        };

        result.map_err(|e| e.retagged(tag).resolved(&config))
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        _fields: &'static [&'static str],
        visitor: V
    ) -> Result<V::Value> {
        use crate::value::magic::*;

        let (config, tag) = (self.config, self.value.tag());
        let result = match name {
            Value::NAME => Value::deserialize_from(self, visitor),
            RelativePathBuf::NAME => RelativePathBuf::deserialize_from(self, visitor),
            Tagged::<()>::NAME => Tagged::<()>::deserialize_from(self, visitor),
            // SelectedProfile::NAME => SelectedProfile::deserialize_from(self, visitor),
            _ => self.deserialize_any(visitor)
        };

        result.map_err(|e| e.retagged(tag).resolved(config))
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        v: V,
    ) -> Result<V::Value> {
        use serde::de::value::MapAccessDeserializer;

        let (config, tag) = (self.config, self.value.tag());
        let result = match self.value {
            Value::String(_, s) => v.visit_enum((&**s).into_deserializer()),
            Value::Dict(_, ref map) => {
                let maker = |v| Self::from(self.config, v);
                let map_access = MapDe::new(map, maker);
                v.visit_enum(MapAccessDeserializer::new(map_access))
            }
            Value::Num(_, n) if n.to_u32().is_some() => {
                let tag = n.to_u32().unwrap();
                v.visit_enum(tag.into_deserializer())
            }
            _ => self.deserialize_any(v)
        };

        result.map_err(|e| e.retagged(tag).resolved(&config))
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_newtype_struct(self)
    }

    fn is_human_readable(&self) -> bool {
        let val = self.readable.get();
        self.readable.set(!val);
        val
    }

    apply_then_forward_to_deserialize_any! {
        deserialize_bool =>
            |de| I::interpret_as_bool(de.value),
            |e| e.retagged(de.value.tag()).resolved(de.config),
        deserialize_u8, deserialize_u16, deserialize_u32, deserialize_u64,
        deserialize_i8, deserialize_i16, deserialize_i32, deserialize_i64,
        deserialize_f32, deserialize_f64 =>
            |de| I::interpret_as_num(de.value),
            |e| e.retagged(de.value.tag()).resolved(de.config),
    }

    serde::forward_to_deserialize_any! {
        char str
        string seq bytes byte_buf map unit
        ignored_any unit_struct tuple_struct tuple identifier
    }
}

use std::collections::btree_map::Iter;

pub struct MapDe<'m, D, F: Fn(&'m Value) -> D> {
    iter: Iter<'m, String, Value>,
    pair: Option<(&'m String, &'m Value)>,
    make_deserializer: F,
}

impl<'m, D, F: Fn(&'m Value) -> D> MapDe<'m, D, F> {
    pub fn new(map: &'m Dict, maker: F) -> Self {
        MapDe { iter: map.iter(), pair: None, make_deserializer: maker }
    }
}

impl<'m, 'de, D, F> de::MapAccess<'de> for MapDe<'m, D, F>
    where D: Deserializer<'de, Error = Error>, F: Fn(&'m Value) -> D,
{
    type Error = Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>>
        where K: de::DeserializeSeed<'de>
    {
        if let Some((k, v)) = self.iter.next() {
            let result = seed.deserialize(k.as_str().into_deserializer())
                .map_err(|e: Error| e.prefixed(k).retagged(v.tag()))
                .map(Some);

            self.pair = Some((k, v));
            result
        } else {
            Ok(None)
        }
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value>
        where V: de::DeserializeSeed<'de>
    {
        let (key, value) = self.pair.take().expect("visit_value called before visit_key");
        let tag = value.tag();
        seed.deserialize((self.make_deserializer)(value))
            .map_err(|e: Error| e.prefixed(key).retagged(tag))
    }
}

pub struct SeqDe<'v, D, F: Fn(&'v Value) -> D> {
    iter: std::iter::Enumerate<std::slice::Iter<'v, Value>>,
    len: usize,
    make_deserializer: F,
}

impl<'v, D, F: Fn(&'v Value) -> D> SeqDe<'v, D, F> {
    pub fn new(seq: &'v [Value], maker: F) -> Self {
        SeqDe { len: seq.len(), iter: seq.iter().enumerate(), make_deserializer: maker }
    }
}

impl<'v, 'de, D, F> de::SeqAccess<'de> for SeqDe<'v, D, F>
    where D: Deserializer<'de, Error = Error>, F: Fn(&'v Value) -> D,
{
    type Error = Error;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>>
        where T: de::DeserializeSeed<'de>
    {
        if let Some((i, item)) = self.iter.next() {
            // item.map_tag(|metadata| metadata.path.push(self.count.to_string()));
            self.len -= 1;
            seed.deserialize((self.make_deserializer)(item))
                .map_err(|e: Error| e.prefixed(&i.to_string()))
                .map(Some)
        } else {
            Ok(None)
        }
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.len)
    }
}

impl<'de> Deserializer<'de> for &Value {
    type Error = Error;

    fn deserialize_any<V>(self, v: V) -> Result<V::Value>
        where V: de::Visitor<'de>
    {
        use Value::*;
        let result = match *self {
            String(_, ref s) => v.visit_str(s),
            Char(_, c) => v.visit_char(c),
            Bool(_, b) => v.visit_bool(b),
            Num(_, n) => n.deserialize_any(v),
            Empty(_, e) => e.deserialize_any(v),
            Dict(_, ref map) => v.visit_map(MapDe::new(map, |v| v)),
            Array(_, ref seq) => v.visit_seq(SeqDe::new(seq, |v| v)),
        };

        result.map_err(|e: Error| e.retagged(self.tag()))
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value>
        where V: Visitor<'de>
    {
        if let Value::Empty(t, val) = self {
            return val.deserialize_any(visitor).map_err(|e: Error| e.retagged(*t));
        }

        visitor.visit_some(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        v: V,
    ) -> Result<V::Value> {
        use serde::de::value::MapAccessDeserializer;

        let result = match self {
            Value::String(_, s) => v.visit_enum((&**s).into_deserializer()),
            Value::Dict(_, ref map) => {
                let map_access = MapDe::new(map, |v| v);
                v.visit_enum(MapAccessDeserializer::new(map_access))
            }
            Value::Num(_, n) if n.to_u32().is_some() => {
                let tag = n.to_u32().unwrap();
                v.visit_enum(tag.into_deserializer())
            }
            _ => self.deserialize_any(v)
        };

        result.map_err(|e: Error| e.retagged(self.tag()))
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_newtype_struct(self)
    }

    serde::forward_to_deserialize_any! {
        bool u8 u16 u32 u64 i8 i16 i32 i64 f32 f64 char str
        string seq bytes byte_buf map unit struct
        ignored_any unit_struct tuple_struct tuple identifier
    }
}

macro_rules! int_try {
    ($n:expr; $o:ty => $t:ty => $($r:tt)*) => (
        if ($n as $t as $o) == $n { return $($r)*($n as $t); }
    )
}

impl<'de> Deserializer<'de> for Num {
    type Error = Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value>
        where V: de::Visitor<'de>
    {
        match self {
            Num::U8(n) => visitor.visit_u8(n),
            Num::U16(n) => visitor.visit_u16(n),
            Num::U32(n) => visitor.visit_u32(n),
            Num::U64(n) => visitor.visit_u64(n),
            Num::U128(n) => visitor.visit_u128(n),
            Num::I8(n) => visitor.visit_i8(n),
            Num::I16(n) => visitor.visit_i16(n),
            Num::I32(n) => visitor.visit_i32(n),
            Num::I64(n) => visitor.visit_i64(n),
            Num::I128(n) => visitor.visit_i128(n),
            Num::F32(n) => visitor.visit_f32(n),
            Num::F64(n) => visitor.visit_f64(n),
            Num::ISize(n) => {
                int_try!(n; isize => i8 => visitor.visit_i8);
                int_try!(n; isize => i16 => visitor.visit_i16);
                int_try!(n; isize => i32 => visitor.visit_i32);
                int_try!(n; isize => i64 => visitor.visit_i64);
                int_try!(n; isize => i128 => visitor.visit_i128);
                Err(Kind::ISizeOutOfRange(n).into())
            }
            Num::USize(n) => {
                int_try!(n; usize => u8 => visitor.visit_u8);
                int_try!(n; usize => u16 => visitor.visit_u16);
                int_try!(n; usize => u32 => visitor.visit_u32);
                int_try!(n; usize => u64 => visitor.visit_u64);
                int_try!(n; usize => u128 => visitor.visit_u128);
                Err(Kind::USizeOutOfRange(n).into())
            }
        }
    }

    serde::forward_to_deserialize_any! {
        bool u8 u16 u32 u64 i8 i16 i32 i64 f32 f64 char str string seq enum
        bytes byte_buf map struct unit newtype_struct
        ignored_any unit_struct tuple_struct tuple option identifier
    }
}

impl<'de> Deserializer<'de> for Empty {
    type Error = Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value>
        where V: de::Visitor<'de>
    {
        match self {
            Empty::Unit => visitor.visit_unit(),
            Empty::None => visitor.visit_none(),
        }
    }

    serde::forward_to_deserialize_any! {
        bool u8 u16 u32 u64 i8 i16 i32 i64 f32 f64 char str string seq enum
        bytes byte_buf map struct unit newtype_struct
        ignored_any unit_struct tuple_struct tuple option identifier
    }
}

/// Marker trait for "magic" values. Primarily for use with [`Either`].
impl Value {
    const NAME: &'static str = "___figment_value";

    const FIELDS: &'static [&'static str] = &[
        "___figment_value_id", "___figment_value_value"
    ];

    fn deserialize_from<'de: 'c, 'c, V: de::Visitor<'de>, I: Interpreter>(
        de: ConfiguredValueDe<'c, I>,
        visitor: V
    ) -> Result<V::Value> {
        let mut map = Dict::new();
        map.insert(Self::FIELDS[0].into(), de.value.tag().into());
        map.insert(Self::FIELDS[1].into(), de.value.clone());
        visitor.visit_map(MapDe::new(&map, |v| ConfiguredValueDe::<'_, I>::from(de.config, v)))
    }
}

#[derive(Debug)]
struct RawValue(Value);

impl<'de> Deserialize<'de> for RawValue {
    fn deserialize<D: Deserializer<'de>>(de: D) -> result::Result<Self, D::Error> {
        de.deserialize_any(ValueVisitor).map(RawValue)
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(de: D) -> result::Result<Value, D::Error> {
        // Total hack to "fingerprint" our deserializer by checking if
        // human_readable changes, which does for ours but shouldn't for others.
        let (a, b) = (de.is_human_readable(), de.is_human_readable());
        if a != b {
            de.deserialize_struct(Value::NAME, Value::FIELDS, ValueVisitor)
        } else {
            de.deserialize_any(ValueVisitor)
        }
    }
}

pub struct ValueVisitor;

macro_rules! visit_fn {
    ($name:ident: $T:ty => $V:path) => (
        #[inline]
        fn $name<E: de::Error>(self, v: $T) -> result::Result<Self::Value, E> {
            Ok(v.into())
        }
    )
}

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any valid figment value")
    }

    visit_fn!(visit_bool: bool => Value::Bool);
    visit_fn!(visit_char: char => Value::Char);
    visit_fn!(visit_str: &str => Value::String);
    visit_fn!(visit_string: String => Value::String);

    visit_fn!(visit_u8: u8 => Num::U8);
    visit_fn!(visit_u16: u16 => Num::U16);
    visit_fn!(visit_u32: u32 => Num::U32);
    visit_fn!(visit_u64: u64 => Num::U64);
    visit_fn!(visit_u128: u128 => Num::U128);

    visit_fn!(visit_i8: i8 => Num::I8);
    visit_fn!(visit_i16: i16 => Num::I16);
    visit_fn!(visit_i32: i32 => Num::I32);
    visit_fn!(visit_i64: i64 => Num::I64);
    visit_fn!(visit_i128: i128 => Num::I128);

    visit_fn!(visit_f32: f32 => Num::F32);
    visit_fn!(visit_f64: f64 => Num::F64);

    fn visit_seq<A>(self, mut seq: A) -> result::Result<Self::Value, A::Error>
        where A: SeqAccess<'de>
    {
        let mut array: Vec<Value> = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(elem) = seq.next_element()? {
            array.push(elem);
        }

        Ok(array.into())
    }

    fn visit_map<A>(self, mut map: A) -> result::Result<Self::Value, A::Error>
        where A: MapAccess<'de>
    {
        let mut dict = Dict::new();
        let mut id: Option<Tag> = None;
        let mut raw_val: Option<RawValue> = None;
        while let Some(key) = map.next_key()? {
            if key == Value::FIELDS[0] {
                id = Some(map.next_value()?);
            } else if key == Value::FIELDS[1] {
                raw_val = Some(map.next_value()?);
            }  else {
                dict.insert(key, map.next_value()?);
            }
        }

        if let Some(mut value) = raw_val {
            if let Some(id) = id {
                value.0.map_tag(|t| *t = id);
            }

            return Ok(value.0);
        }

        Ok(dict.into())
    }

    fn visit_enum<A: de::EnumAccess<'de>>(self, data: A) -> result::Result<Self::Value, A::Error> {
        let (tag, variant) = data.variant::<String>()?;
        Ok(crate::util::nest(&tag, variant.newtype_variant()?))
    }

    fn visit_none<E: de::Error>(self) -> result::Result<Self::Value, E> {
        Ok(Empty::None.into())
    }

    fn visit_some<D>(self, deserializer: D) -> result::Result<Self::Value, D::Error>
        where D: Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }

    fn visit_unit<E: de::Error>(self) -> result::Result<Self::Value, E> {
        Ok(Empty::Unit.into())
    }
}
