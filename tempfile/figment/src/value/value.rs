use std::collections::BTreeMap;
use std::num::{ParseFloatError, ParseIntError};
use std::str::{FromStr, Split};

use serde::Serialize;

use crate::value::{Tag, ValueSerializer, magic::Either};
use crate::error::{Error, Actual};

/// An alias to the type of map used in [`Value::Dict`].
pub type Map<K, V> = BTreeMap<K, V>;

/// An alias to a [`Map`] from `String` to [`Value`]s.
pub type Dict = Map<String, Value>;

/// An enum representing all possible figment value variants.
///
/// Note that `Value` implements `From<T>` for all reasonable `T`:
///
/// ```
/// use figment::value::Value;
///
/// let v = Value::from("hello");
/// assert_eq!(v.as_str(), Some("hello"));
/// ```
#[derive(Clone)]
pub enum Value {
    /// A string.
    String(Tag, String),
    /// A character.
    Char(Tag, char),
    /// A boolean.
    Bool(Tag, bool),
    /// A numeric value.
    Num(Tag, Num),
    /// A value with no value.
    Empty(Tag, Empty),
    /// A dictionary: a map from `String` to `Value`.
    Dict(Tag, Dict),
    /// A sequence/array/vector.
    Array(Tag, Vec<Value>),
}

macro_rules! conversion_fn {
    ($RT:ty, $([$star:tt])? $Variant:ident => $T:ty, $fn_name:ident) => {
        conversion_fn!(
            concat!(
                "Converts `self` into a `", stringify!($T), "` if `self` is a \
                `Value::", stringify!($Variant), "`.\n\n",
                "# Example\n\n",
                "```\n",
                "use figment::value::Value;\n\n",
                "let value: Value = 123.into();\n",
                "let converted = value.", stringify!($fn_name), "();\n",
                "```"
            ),
            $RT, $([$star])? $Variant => $T, $fn_name
        );
    };

    ($doc:expr, $RT:ty, $([$star:tt])? $Variant:ident => $T:ty, $fn_name:ident) => {
        #[doc = $doc]
        pub fn $fn_name(self: $RT) -> Option<$T> {
            match $($star)? self {
                Value::$Variant(_, v) => Some(v),
                _ => None
            }
        }
    };
}

impl Value {
    /// Serialize a `Value` from any `T: Serialize`.
    ///
    /// ```
    /// use figment::value::{Value, Empty};
    ///
    /// let value = Value::serialize(10i8).unwrap();
    /// assert_eq!(value.to_i128(), Some(10));
    ///
    /// let value = Value::serialize(()).unwrap();
    /// assert_eq!(value, Empty::Unit.into());
    ///
    /// let value = Value::serialize(vec![4, 5, 6]).unwrap();
    /// assert_eq!(value, vec![4, 5, 6].into());
    /// ```
    pub fn serialize<T: Serialize>(value: T) -> Result<Self, Error> {
        value.serialize(ValueSerializer)
    }

    /// Deserialize `self` into any deserializable `T`.
    ///
    /// ```
    /// use figment::value::Value;
    ///
    /// let value = Value::from("hello");
    /// let string: String = value.deserialize().unwrap();
    /// assert_eq!(string, "hello");
    /// ```
    pub fn deserialize<'de, T: serde::Deserialize<'de>>(&self) -> Result<T, Error> {
        T::deserialize(self)
    }

    /// Looks up and returns the value at path `path`, where `path` is of the
    /// form `a.b.c` where `a`, `b`, and `c` are keys to dictionaries. If the
    /// key is empty, simply returns `self`. If the key is not empty and `self`
    /// or any of the values for non-leaf keys in the path are not dictionaries,
    /// returns `None`.
    ///
    /// This method consumes `self`. See [`Value::find_ref()`] for a
    /// non-consuming variant.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::{value::Value, util::map};
    ///
    /// let value = Value::from(map! {
    ///     "apple" => map! {
    ///         "bat" => map! {
    ///             "pie" => 4usize,
    ///         },
    ///         "cake" => map! {
    ///             "pumpkin" => 10usize,
    ///         }
    ///     }
    /// });
    ///
    /// assert!(value.clone().find("apple").is_some());
    /// assert!(value.clone().find("apple.bat").is_some());
    /// assert!(value.clone().find("apple.cake").is_some());
    ///
    /// assert_eq!(value.clone().find("apple.bat.pie").unwrap().to_u128(), Some(4));
    /// assert_eq!(value.clone().find("apple.cake.pumpkin").unwrap().to_u128(), Some(10));
    ///
    /// assert!(value.clone().find("apple.pie").is_none());
    /// assert!(value.clone().find("pineapple").is_none());
    /// ```
    pub fn find(self, path: &str) -> Option<Value> {
        fn find(mut keys: Split<char>, value: Value) -> Option<Value> {
            match keys.next() {
                Some(k) if !k.is_empty() => find(keys, value.into_dict()?.remove(k)?),
                Some(_) | None => Some(value)
            }
        }

        find(path.split('.'), self)
    }

    /// Exactly like [`Value::find()`] but does not consume `self`,
    /// returning a reference to the found value, if any, instead.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::{value::Value, util::map};
    ///
    /// let value = Value::from(map! {
    ///     "apple" => map! {
    ///         "bat" => map! {
    ///             "pie" => 4usize,
    ///         },
    ///         "cake" => map! {
    ///             "pumpkin" => 10usize,
    ///         }
    ///     }
    /// });
    ///
    /// assert!(value.find_ref("apple").is_some());
    /// assert!(value.find_ref("apple.bat").is_some());
    /// assert!(value.find_ref("apple.cake").is_some());
    ///
    /// assert_eq!(value.find_ref("apple.bat.pie").unwrap().to_u128(), Some(4));
    /// assert_eq!(value.find_ref("apple.cake.pumpkin").unwrap().to_u128(), Some(10));
    ///
    /// assert!(value.find_ref("apple.pie").is_none());
    /// assert!(value.find_ref("pineapple").is_none());
    /// ```
    pub fn find_ref<'a>(&'a self, path: &str) -> Option<&'a Value> {
        fn find<'a, 'v>(mut keys: Split<'a, char>, value: &'v Value) -> Option<&'v Value> {
            match keys.next() {
                Some(k) if !k.is_empty() => find(keys, value.as_dict()?.get(k)?),
                Some(_) | None => Some(value)
            }
        }

        find(path.split('.'), self)
    }

    /// Returns the [`Tag`] applied to this value.
    ///
    /// ```
    /// use figment::{Figment, Profile, value::Value, util::map};
    ///
    /// let map: Value = Figment::from(("key", "value")).extract().unwrap();
    /// let value = map.find_ref("key").expect("value");
    /// assert_eq!(value.as_str(), Some("value"));
    /// assert!(!value.tag().is_default());
    /// assert_eq!(value.tag().profile(), Some(Profile::Global));
    ///
    /// let map: Value = Figment::from(("key", map!["key2" => 123])).extract().unwrap();
    /// let value = map.find_ref("key.key2").expect("value");
    /// assert_eq!(value.to_i128(), Some(123));
    /// assert!(!value.tag().is_default());
    /// assert_eq!(value.tag().profile(), Some(Profile::Global));
    /// ```
    pub fn tag(&self) -> Tag {
        match *self {
            Value::String(tag, ..) => tag,
            Value::Char(tag, ..) => tag,
            Value::Bool(tag, ..) => tag,
            Value::Num(tag, ..) => tag,
            Value::Dict(tag, ..) => tag,
            Value::Array(tag, ..) => tag,
            Value::Empty(tag, ..) => tag,
        }
    }

    conversion_fn!(&Value, String => &str, as_str);
    conversion_fn!(Value, String => String, into_string);
    conversion_fn!(&Value, [*]Char => char, to_char);
    conversion_fn!(&Value, [*]Bool => bool, to_bool);
    conversion_fn!(&Value, [*]Num => Num, to_num);
    conversion_fn!(&Value, [*]Empty => Empty, to_empty);
    conversion_fn!(&Value, Dict => &Dict, as_dict);
    conversion_fn!(Value, Dict => Dict, into_dict);
    conversion_fn!(&Value, Array => &[Value], as_array);
    conversion_fn!(Value, Array => Vec<Value>, into_array);

    /// Converts `self` into a `u128` if `self` is an unsigned `Value::Num`
    /// variant.
    ///
    /// # Example
    ///
    /// ```
    /// use figment::value::Value;
    ///
    /// let value: Value = 123u8.into();
    /// let converted = value.to_u128();
    /// assert_eq!(converted, Some(123));
    /// ```
    pub fn to_u128(&self) -> Option<u128> {
        self.to_num()?.to_u128()
    }

    /// Converts `self` into an `i128` if `self` is an signed `Value::Num`
    /// variant.
    ///
    /// # Example
    ///
    /// ```
    /// use figment::value::Value;
    ///
    /// let value: Value = 123i8.into();
    /// let converted = value.to_i128();
    /// assert_eq!(converted, Some(123));
    ///
    /// let value: Value = Value::from(5000i64);
    /// assert_eq!(value.to_i128(), Some(5000i128));
    /// ```
    pub fn to_i128(&self) -> Option<i128> {
        self.to_num()?.to_i128()
    }

    /// Converts `self` into an `f64` if `self` is either a [`Num::F32`] or
    /// [`Num::F64`].
    ///
    /// # Example
    ///
    /// ```
    /// use figment::value::Value;
    ///
    /// let value: Value = 7.0f32.into();
    /// let converted = value.to_f64();
    /// assert_eq!(converted, Some(7.0f64));
    ///
    /// let value: Value = Value::from(7.0f64);
    /// assert_eq!(value.to_f64(), Some(7.0f64));
    /// ```
    pub fn to_f64(&self) -> Option<f64> {
        self.to_num()?.to_f64()
    }

    /// Converts `self` to a `bool` if it is a [`Value::Bool`], or if it is a
    /// [`Value::String`] or a [`Value::Num`] with a boolean interpretation.
    ///
    /// The case-insensitive strings "true", "yes", "1", and "on", and the
    /// signed or unsigned integers `1` are interpreted as `true`.
    ///
    /// The case-insensitive strings "false", "no", "0", and "off", and the
    /// signed or unsigned integers `0` are interpreted as false.
    ///
    /// # Example
    ///
    /// ```
    /// use figment::value::Value;
    ///
    /// let value = Value::from(true);
    /// assert_eq!(value.to_bool_lossy(), Some(true));
    ///
    /// let value = Value::from(1);
    /// assert_eq!(value.to_bool_lossy(), Some(true));
    ///
    /// let value = Value::from("YES");
    /// assert_eq!(value.to_bool_lossy(), Some(true));
    ///
    /// let value = Value::from(false);
    /// assert_eq!(value.to_bool_lossy(), Some(false));
    ///
    /// let value = Value::from(0);
    /// assert_eq!(value.to_bool_lossy(), Some(false));
    ///
    /// let value = Value::from("no");
    /// assert_eq!(value.to_bool_lossy(), Some(false));
    ///
    /// let value = Value::from("hello");
    /// assert_eq!(value.to_bool_lossy(), None);
    /// ```
    pub fn to_bool_lossy(&self) -> Option<bool> {
        match self {
            Value::Bool(_, b) => Some(*b),
            Value::Num(_, num) => match num.to_u128_lossy() {
                Some(0) => Some(false),
                Some(1) => Some(true),
                _ => None
            }
            Value::String(_, s) => {
                const TRUE: &[&str] = &["true", "yes", "1", "on"];
                const FALSE: &[&str] = &["false", "no", "0", "off"];

                if TRUE.iter().any(|v| uncased::eq(v, s)) {
                    Some(true)
                } else if FALSE.iter().any(|v| uncased::eq(v, s)) {
                    Some(false)
                } else {
                    None
                }
            },
            _ => None,
        }
    }

    /// Converts `self` to a [`Num`] if it is a [`Value::Num`] or if it is a
    /// [`Value::String`] that parses as a `usize` ([`Num::USize`]), `isize`
    /// ([`Num::ISize`]), or `f64` ([`Num::F64`]), in that order of precendence.
    ///
    /// # Examples
    ///
    /// ```
    /// use figment::value::{Value, Num};
    ///
    /// let value = Value::from(7_i32);
    /// assert_eq!(value.to_num_lossy(), Some(Num::I32(7)));
    ///
    /// let value = Value::from("7");
    /// assert_eq!(value.to_num_lossy(), Some(Num::U8(7)));
    ///
    /// let value = Value::from("-7000");
    /// assert_eq!(value.to_num_lossy(), Some(Num::I16(-7000)));
    ///
    /// let value = Value::from("7000.5");
    /// assert_eq!(value.to_num_lossy(), Some(Num::F64(7000.5)));
    /// ```
    pub fn to_num_lossy(&self) -> Option<Num> {
        match self {
            Value::Num(_, num) => Some(*num),
            Value::String(_, s) => s.parse().ok(),
            _ => None,
        }
    }

    /// Converts `self` into the corresponding [`Actual`].
    ///
    /// See also [`Num::to_actual()`] and [`Empty::to_actual()`], which are
    /// called internally by this method.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::{value::Value, error::Actual};
    ///
    /// assert_eq!(Value::from('a').to_actual(), Actual::Char('a'));
    /// assert_eq!(Value::from(&[1, 2, 3]).to_actual(), Actual::Seq);
    /// ```
    pub fn to_actual(&self) -> Actual {
        match self {
            Value::String(_, s) => Actual::Str(s.into()),
            Value::Char(_, c) => Actual::Char(*c),
            Value::Bool(_, b) => Actual::Bool(*b),
            Value::Num(_, n) => n.to_actual(),
            Value::Empty(_, e) => e.to_actual(),
            Value::Dict(_, _) => Actual::Map,
            Value::Array(_, _) => Actual::Seq,
        }
    }

    pub(crate) fn tag_mut(&mut self) -> &mut Tag {
        match self {
            Value::String(tag, ..) => tag,
            Value::Char(tag, ..) => tag,
            Value::Bool(tag, ..) => tag,
            Value::Num(tag, ..) => tag,
            Value::Dict(tag, ..) => tag,
            Value::Array(tag, ..) => tag,
            Value::Empty(tag, ..) => tag,
        }
    }

    pub(crate) fn map_tag<F>(&mut self, mut f: F)
        where F: FnMut(&mut Tag) + Copy
    {
        if *self.tag_mut() == Tag::Default {
            f(self.tag_mut());
        }

        match self {
            Value::Dict(_, v) => v.iter_mut().for_each(|(_, v)| v.map_tag(f)),
            Value::Array(_, v) => v.iter_mut().for_each(|v| v.map_tag(f)),
            _ => { /* already handled */ }
        }
    }
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String(_, v) => f.debug_tuple("String").field(v).finish(),
            Self::Char(_, v) => f.debug_tuple("Char").field(v).finish(),
            Self::Bool(_, v) => f.debug_tuple("Bool").field(v).finish(),
            Self::Num(_, v) => f.debug_tuple("Num").field(v).finish(),
            Self::Empty(_, v) => f.debug_tuple("Empty").field(v).finish(),
            Self::Dict(_, v) => f.debug_tuple("Dict").field(v).finish(),
            Self::Array(_, v) => f.debug_tuple("Array").field(v).finish(),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::String(_, v1), Value::String(_, v2)) => v1 == v2,
            (Value::Char(_, v1), Value::Char(_, v2)) => v1 == v2,
            (Value::Bool(_, v1), Value::Bool(_, v2)) => v1 == v2,
            (Value::Num(_, v1), Value::Num(_, v2)) => v1 == v2,
            (Value::Empty(_, v1), Value::Empty(_, v2)) => v1 == v2,
            (Value::Dict(_, v1), Value::Dict(_, v2)) => v1 == v2,
            (Value::Array(_, v1), Value::Array(_, v2)) => v1 == v2,
            _ => false,
        }
    }
}

macro_rules! impl_from_array {
    ($($N:literal),*) => ($(impl_from_array!(@$N);)*);
    (@$N:literal) => (
        impl<'a, T: Into<Value> + Clone> From<&'a [T; $N]> for Value {
            #[inline(always)]
            fn from(value: &'a [T; $N]) -> Value {
                Value::from(&value[..])
            }
        }
    )
}

impl_from_array!(1, 2, 3, 4, 5, 6, 7, 8);

impl From<&str> for Value {
    fn from(value: &str) -> Value {
        Value::String(Tag::Default, value.to_string())
    }
}

impl<'a, T: Into<Value> + Clone> From<&'a [T]> for Value {
    fn from(value: &'a [T]) -> Value {
        Value::Array(Tag::Default, value.iter().map(|v| v.clone().into()).collect())
    }
}

impl<'a, T: Into<Value>> From<Vec<T>> for Value {
    fn from(vec: Vec<T>) -> Value {
        let vector = vec.into_iter().map(|v| v.into()).collect();
        Value::Array(Tag::Default, vector)
    }
}

impl<K: AsRef<str>, V: Into<Value>> From<Map<K, V>> for Value {
    fn from(map: Map<K, V>) -> Value {
        let dict: Dict = map.into_iter()
            .map(|(k, v)| (k.as_ref().to_string(), v.into()))
            .collect();

        Value::Dict(Tag::Default, dict)
    }
}

macro_rules! impl_from_for_value {
    ($($T:ty: $V:ident),*) => ($(
        impl From<$T> for Value {
            fn from(value: $T) -> Value { Value::$V(Tag::Default, value.into()) }
        }
    )*)
}

macro_rules! try_convert {
    ($n:expr => $($T:ty),*) => {$(
        if let Ok(n) = <$T as std::convert::TryFrom<_>>::try_from($n) {
            return Ok(n.into());
        }
    )*}
}

impl FromStr for Num {
    type Err = Either<ParseIntError, ParseFloatError>;

    fn from_str(string: &str) -> Result<Self, Self::Err> {
        let string = string.trim();
        if string.contains('.') {
            if string.len() <= (f32::DIGITS as usize + 1) {
                Ok(string.parse::<f32>().map_err(Either::Right)?.into())
            } else {
                Ok(string.parse::<f64>().map_err(Either::Right)?.into())
            }
        } else if string.starts_with('-') {
            let int = string.parse::<i128>().map_err(Either::Left)?;
            try_convert![int => i8, i16, i32, i64];
            Ok(int.into())
        } else {
            let uint = string.parse::<u128>().map_err(Either::Left)?;
            try_convert![uint => u8, u16, u32, u64];
            Ok(uint.into())
        }
    }
}

impl_from_for_value! {
    String: String, char: Char, bool: Bool,
    u8: Num, u16: Num, u32: Num, u64: Num, u128: Num, usize: Num,
    i8: Num, i16: Num, i32: Num, i64: Num, i128: Num, isize: Num,
    f32: Num, f64: Num, Num: Num, Empty: Empty
}

/// A signed or unsigned numeric value.
#[derive(Debug, Clone, Copy)]
pub enum Num {
    /// An 8-bit unsigned integer.
    U8(u8),
    /// A 16-bit unsigned integer.
    U16(u16),
    /// A 32-bit unsigned integer.
    U32(u32),
    /// A 64-bit unsigned integer.
    U64(u64),
    /// A 128-bit unsigned integer.
    U128(u128),
    /// An unsigned integer of platform width.
    USize(usize),
    /// An 8-bit signed integer.
    I8(i8),
    /// A 16-bit signed integer.
    I16(i16),
    /// A 32-bit signed integer.
    I32(i32),
    /// A 64-bit signed integer.
    I64(i64),
    /// A 128-bit signed integer.
    I128(i128),
    /// A signed integer of platform width.
    ISize(isize),
    /// A 32-bit wide float.
    F32(f32),
    /// A 64-bit wide float.
    F64(f64),
}

impl Num {
    /// Converts `self` into a `u32` if `self` is an unsigned variant with `<=
    /// 32` bits.
    ///
    /// # Example
    ///
    /// ```
    /// use figment::value::Num;
    ///
    /// let num: Num = 123u8.into();
    /// assert_eq!(num.to_u32(), Some(123));
    ///
    /// let num: Num = (u32::max_value() as u64 + 1).into();
    /// assert_eq!(num.to_u32(), None);
    /// ```
    pub fn to_u32(self) -> Option<u32> {
        Some(match self {
            Num::U8(v) => v as u32,
            Num::U16(v) => v as u32,
            Num::U32(v) => v as u32,
            _ => return None,
        })
    }

    /// Converts `self` into a `u128` if `self` is an unsigned variant.
    ///
    /// # Example
    ///
    /// ```
    /// use figment::value::Num;
    ///
    /// let num: Num = 123u8.into();
    /// assert_eq!(num.to_u128(), Some(123));
    /// ```
    pub fn to_u128(self) -> Option<u128> {
        Some(match self {
            Num::U8(v) => v as u128,
            Num::U16(v) => v as u128,
            Num::U32(v) => v as u128,
            Num::U64(v) => v as u128,
            Num::U128(v) => v as u128,
            Num::USize(v) => v as u128,
            _ => return None,
        })
    }

    /// Converts `self` into a `u128` if it is non-negative, even if `self` is
    /// of a signed variant.
    ///
    /// # Example
    ///
    /// ```
    /// use figment::value::Num;
    ///
    /// let num: Num = 123u8.into();
    /// assert_eq!(num.to_u128_lossy(), Some(123));
    ///
    /// let num: Num = 123i8.into();
    /// assert_eq!(num.to_u128_lossy(), Some(123));
    /// ```
    pub fn to_u128_lossy(self) -> Option<u128> {
        Some(match self {
            Num::U8(v) => v as u128,
            Num::U16(v) => v as u128,
            Num::U32(v) => v as u128,
            Num::U64(v) => v as u128,
            Num::U128(v) => v as u128,
            Num::USize(v) => v as u128,
            Num::I8(v) if v >= 0 => v as u128,
            Num::I16(v) if v >= 0 => v as u128,
            Num::I32(v) if v >= 0 => v as u128,
            Num::I64(v) if v >= 0 => v as u128,
            Num::I128(v) if v >= 0 => v as u128,
            Num::ISize(v) if v >= 0 => v as u128,
            _ => return None,
        })
    }

    /// Converts `self` into an `i128` if `self` is a signed `Value::Num`
    /// variant.
    ///
    /// # Example
    ///
    /// ```
    /// use figment::value::Num;
    ///
    /// let num: Num = 123i8.into();
    /// assert_eq!(num.to_i128(), Some(123));
    /// ```
    pub fn to_i128(self) -> Option<i128> {
        Some(match self {
            Num::I8(v) => v as i128,
            Num::I16(v) => v as i128,
            Num::I32(v) => v as i128,
            Num::I64(v) => v as i128,
            Num::I128(v) => v as i128,
            Num::ISize(v) => v as i128,
            _ => return None,
        })
    }

    /// Converts `self` into an `f64` if `self` is either a [`Num::F32`] or
    /// [`Num::F64`].
    ///
    /// # Example
    ///
    /// ```
    /// use figment::value::Num;
    ///
    /// let num: Num = 3.0f32.into();
    /// assert_eq!(num.to_f64(), Some(3.0f64));
    /// ```
    pub fn to_f64(&self) -> Option<f64> {
        Some(match *self {
            Num::F32(v) => v as f64,
            Num::F64(v) => v as f64,
            _ => return None,
        })
    }

    /// Converts `self` into an [`Actual`]. All unsigned variants return
    /// [`Actual::Unsigned`], signed variants [`Actual::Signed`], and float
    /// variants [`Actual::Float`]. Values exceeding the bit-width of the target
    /// [`Actual`] are truncated.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::{value::Num, error::Actual};
    ///
    /// assert_eq!(Num::U8(10).to_actual(), Actual::Unsigned(10));
    /// assert_eq!(Num::U64(2380).to_actual(), Actual::Unsigned(2380));
    ///
    /// assert_eq!(Num::I8(127).to_actual(), Actual::Signed(127));
    /// assert_eq!(Num::ISize(23923).to_actual(), Actual::Signed(23923));
    ///
    /// assert_eq!(Num::F32(2.5).to_actual(), Actual::Float(2.5));
    /// assert_eq!(Num::F64(2.103).to_actual(), Actual::Float(2.103));
    /// ```
    pub fn to_actual(&self) -> Actual {
        match *self {
            Num::U8(v) => Actual::Unsigned(v as u128),
            Num::U16(v) => Actual::Unsigned(v as u128),
            Num::U32(v) => Actual::Unsigned(v as u128),
            Num::U64(v) => Actual::Unsigned(v as u128),
            Num::U128(v) => Actual::Unsigned(v as u128),
            Num::USize(v) => Actual::Unsigned(v as u128),
            Num::I8(v) => Actual::Signed(v as i128),
            Num::I16(v) => Actual::Signed(v as i128),
            Num::I32(v) => Actual::Signed(v as i128),
            Num::I64(v) => Actual::Signed(v as i128),
            Num::I128(v) => Actual::Signed(v as i128),
            Num::ISize(v) => Actual::Signed(v as i128),
            Num::F32(v) => Actual::Float(v as f64),
            Num::F64(v) => Actual::Float(v as f64),
        }
    }
}

impl PartialEq for Num {
    fn eq(&self, other: &Self) -> bool {
        self.to_actual() == other.to_actual()
    }
}

macro_rules! impl_from_for_num_value {
    ($($T:ty: $V:ident),*) => ($(
        impl From<$T> for Num {
            fn from(value: $T) -> Num {
                Num::$V(value)
            }
        }
    )*)
}

impl_from_for_num_value! {
    u8: U8, u16: U16, u32: U32, u64: U64, u128: U128, usize: USize,
    i8: I8, i16: I16, i32: I32, i64: I64, i128: I128, isize: ISize,
    f32: F32, f64: F64
}

/// A value with no value: `None` or `Unit`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Empty {
    /// Like `Option::None`.
    None,
    /// Like `()`.
    Unit
}

impl Empty {
    /// Converts `self` into an [`Actual`].
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::{value::Empty, error::Actual};
    ///
    /// assert_eq!(Empty::None.to_actual(), Actual::Option);
    /// assert_eq!(Empty::Unit.to_actual(), Actual::Unit);
    /// ```
    pub fn to_actual(&self) -> Actual {
        match self {
            Empty::None => Actual::Option,
            Empty::Unit => Actual::Unit,
        }
    }
}
