use serde::{ser, Serialize, Serializer};

use crate::error::{Error, Kind};
use crate::value::{Value, Dict, Num, Empty};

type Result<T> = std::result::Result<T, Error>;

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, ser: S) -> std::result::Result<S::Ok, S::Error> {
        use ser::{SerializeSeq, SerializeMap};

        match self {
            Value::String(_, v) => ser.serialize_str(v),
            Value::Char(_, v) => ser.serialize_char(*v),
            Value::Bool(_, v) => ser.serialize_bool(*v),
            Value::Num(_, v) => v.serialize(ser),
            Value::Empty(_, v) => v.serialize(ser),
            Value::Dict(_, v) => {
                let mut map = ser.serialize_map(Some(v.len()))?;
                for (key, val) in v {
                    map.serialize_entry(key, val)?;
                }

                map.end()
            }
            Value::Array(_, v) => {
                let mut seq = ser.serialize_seq(Some(v.len()))?;
                for elem in v {
                    seq.serialize_element(elem)?;
                }

                seq.end()
            }
        }
    }
}

impl Serialize for Num {
    fn serialize<S: Serializer>(&self, ser: S) -> std::result::Result<S::Ok, S::Error> {
        match *self {
            Num::U8(v) => ser.serialize_u8(v),
            Num::U16(v) => ser.serialize_u16(v),
            Num::U32(v) => ser.serialize_u32(v),
            Num::U64(v) => ser.serialize_u64(v),
            Num::U128(v) => ser.serialize_u128(v),
            Num::USize(v) => ser.serialize_u64(v as u64),
            Num::I8(v) => ser.serialize_i8(v),
            Num::I16(v) => ser.serialize_i16(v),
            Num::I32(v) => ser.serialize_i32(v),
            Num::I64(v) => ser.serialize_i64(v),
            Num::I128(v) => ser.serialize_i128(v),
            Num::ISize(v) => ser.serialize_i64(v as i64),
            Num::F32(v) => ser.serialize_f32(v),
            Num::F64(v) => ser.serialize_f64(v),
        }
    }
}

impl Serialize for Empty {
    fn serialize<S: Serializer>(&self, ser: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Empty::None => ser.serialize_none(),
            Empty::Unit => ser.serialize_unit(),
        }
    }
}

pub struct ValueSerializer;

macro_rules! serialize_fn {
    ($name:ident: $T:ty => $V:path) => (
        fn $name(self, v: $T) -> Result<Self::Ok> { Ok(v.into()) }
    )
}

pub struct SeqSerializer {
    tag: Option<&'static str>,
    sequence: Vec<Value>
}

pub struct MapSerializer {
    tag: Option<&'static str>,
    keys: Vec<String>,
    values: Vec<Value>
}

impl Serializer for ValueSerializer {
    type Ok = Value;
    type Error = Error;

    type SerializeSeq = SeqSerializer;
    type SerializeTuple = SeqSerializer;
    type SerializeTupleStruct = SeqSerializer;
    type SerializeTupleVariant = SeqSerializer;
    type SerializeMap = MapSerializer;
    type SerializeStruct = MapSerializer;
    type SerializeStructVariant = MapSerializer;

    serialize_fn!(serialize_bool: bool => Value::Bool);
    serialize_fn!(serialize_char: char => Value::Char);
    serialize_fn!(serialize_str: &str => Value::String);

    serialize_fn!(serialize_i8: i8 => Num::I8);
    serialize_fn!(serialize_i16: i16 => Num::I16);
    serialize_fn!(serialize_i32: i32 => Num::I32);
    serialize_fn!(serialize_i64: i64 => Num::I64);
    serialize_fn!(serialize_i128: i128 => Num::I128);

    serialize_fn!(serialize_u8: u8 => Num::U8);
    serialize_fn!(serialize_u16: u16 => Num::U16);
    serialize_fn!(serialize_u32: u32 => Num::U32);
    serialize_fn!(serialize_u64: u64 => Num::U64);
    serialize_fn!(serialize_u128: u128 => Num::U128);

    serialize_fn!(serialize_f32: f32 => Num::F32);
    serialize_fn!(serialize_f64: f64 => Num::F64);

    fn serialize_bytes(self, v: &[u8]) -> Result<Self::Ok> {
        use serde::ser::SerializeSeq;
        let mut seq = self.serialize_seq(Some(v.len()))?;
        for byte in v {
            seq.serialize_element(byte)?;
        }

        seq.end()
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq> {
        Ok(SeqSerializer::new(None, len))
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap> {
        Ok(MapSerializer::new(None, len))
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct> {
        Ok(MapSerializer::new(None, Some(len)))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant> {
        Ok(MapSerializer::new(Some(variant), Some(len)))
    }

    fn serialize_some<T: ?Sized>(self, value: &T) -> Result<Self::Ok>
        where T: Serialize
    {
        value.serialize(self)
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok> {
        self.serialize_str(variant)
    }

    fn serialize_newtype_struct<T: ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok>
        where T: Serialize
    {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok> {
        Ok(crate::util::map![variant => value.serialize(self)?].into())
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant> {
        Ok(SeqSerializer::new(Some(variant), Some(len)))
    }

    fn serialize_none(self) -> Result<Self::Ok> {
        Ok(Empty::None.into())
    }

    fn serialize_unit(self) -> Result<Self::Ok> {
        Ok(Empty::Unit.into())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok> {
        self.serialize_unit()
    }

}

impl SeqSerializer {
    pub fn new(tag: Option<&'static str>, len: Option<usize>) -> Self {
        Self {
            tag,
            sequence: len.map(Vec::with_capacity).unwrap_or_default(),
        }
    }
}

impl<'a> ser::SerializeSeq for SeqSerializer {
    type Ok = Value;
    type Error = Error;

    fn serialize_element<T: ?Sized>(&mut self, value: &T) -> Result<()>
        where T: Serialize
    {
        Ok(self.sequence.push(value.serialize(ValueSerializer)?))
    }

    fn end(self) -> Result<Self::Ok> {
        let value: Value = self.sequence.into();
        match self.tag {
            Some(tag) => Ok(crate::util::map![tag => value].into()),
            None => Ok(value)
        }
    }
}

impl<'a> ser::SerializeTuple for SeqSerializer {
    type Ok = Value;
    type Error = Error;

    fn serialize_element<T: ?Sized>(&mut self, value: &T) -> Result<()>
        where T: Serialize
    {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok> {
        ser::SerializeSeq::end(self)
    }
}

// Same thing but for tuple structs.
impl<'a> ser::SerializeTupleStruct for SeqSerializer {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T: ?Sized>(&mut self, value: &T) -> Result<()>
        where T: Serialize
    {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok> {
        ser::SerializeSeq::end(self)
    }
}

impl<'a> ser::SerializeTupleVariant for SeqSerializer {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T: ?Sized>(&mut self, value: &T) -> Result<()>
        where T: Serialize
    {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok> {
        ser::SerializeSeq::end(self)
    }
}

impl MapSerializer {
    pub fn new(tag: Option<&'static str>, len: Option<usize>) -> Self {
        Self {
            tag,
            keys: len.map(Vec::with_capacity).unwrap_or_default(),
            values: len.map(Vec::with_capacity).unwrap_or_default(),
        }
    }
}

impl<'a> ser::SerializeMap for MapSerializer {
    type Ok = Value;
    type Error = Error;

    fn serialize_key<T: ?Sized>(&mut self, key: &T) -> Result<()>
        where T: Serialize
    {
        match key.serialize(ValueSerializer)? {
            Value::String(_, s) => self.keys.push(s),
            v => return Err(Kind::UnsupportedKey(v.to_actual(), "string".into()).into()),
        };

        Ok(())
    }

    fn serialize_value<T: ?Sized>(&mut self, value: &T) -> Result<()>
        where T: Serialize
    {
        self.values.push(value.serialize(ValueSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok> {
        let iter = self.keys.into_iter().zip(self.values.into_iter());
        let value: Value = iter.collect::<Dict>().into();
        match self.tag {
            Some(tag) => Ok(crate::util::map![tag => value].into()),
            None => Ok(value)
        }
    }
}

impl<'a> ser::SerializeStruct for MapSerializer {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T: ?Sized>(&mut self, key: &'static str, value: &T) -> Result<()>
        where T: Serialize
    {
        ser::SerializeMap::serialize_key(self, key)?;
        ser::SerializeMap::serialize_value(self, value)
    }

    fn end(self) -> Result<Self::Ok> {
        ser::SerializeMap::end(self)
    }
}

impl<'a> ser::SerializeStructVariant for MapSerializer {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T: ?Sized>(&mut self, key: &'static str, value: &T) -> Result<()>
        where T: Serialize
    {
        ser::SerializeMap::serialize_key(self, key)?;
        ser::SerializeMap::serialize_value(self, value)
    }

    fn end(self) -> Result<Self::Ok> {
        ser::SerializeMap::end(self)
    }
}
