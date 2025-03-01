use std::fmt;
use std::sync::atomic::Ordering;

use serde::{de, ser};
use crate::profile::{Profile, ProfileTag};
/// An opaque, unique tag identifying a value's [`Metadata`](crate::Metadata)
/// and profile.
///
/// A `Tag` is retrieved either via [`Tagged`] or [`Value::tag()`]. The
/// corresponding metadata can be retrieved via [`Figment::get_metadata()`] and
/// the profile vile [`Tag::profile()`].
///
/// [`Tagged`]: crate::value::magic::Tagged
/// [`Value::tag()`]: crate::value::Value::tag()
/// [`Figment::get_metadata()`]: crate::Figment::get_metadata()
#[derive(Copy, Clone)]
pub struct Tag(u64);

#[cfg(any(target_pointer_width = "8", target_pointer_width = "16", target_pointer_width = "32"))]
static COUNTER: atomic::Atomic<u64> = atomic::Atomic::new(1);

#[cfg(not(any(target_pointer_width = "8", target_pointer_width = "16", target_pointer_width = "32")))]
static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);


impl Tag {
    /// The default `Tag`. Such a tag will never have associated metadata and
    /// is associated with a profile of `Default`.
    // NOTE: `0` is special! We should never create a default tag via `next()`.
    #[allow(non_upper_case_globals)]
    pub const Default: Tag = Tag(0);

    const PROFILE_TAG_SHIFT: u64 = 62;
    const PROFILE_TAG_MASK: u64 = 0b11 << Self::PROFILE_TAG_SHIFT;

    const METADATA_ID_SHIFT: u64 = 0;
    const METADATA_ID_MASK: u64 = (!Self::PROFILE_TAG_MASK) << Self::METADATA_ID_SHIFT;

    const fn new(metadata_id: u64, profile_tag: ProfileTag) -> Tag {
        let bits = ((metadata_id << Self::METADATA_ID_SHIFT) & Self::METADATA_ID_MASK)
            | ((profile_tag as u64) << Self::PROFILE_TAG_SHIFT) & Self::PROFILE_TAG_MASK;

        Tag(bits)
    }

    // Returns a tag with a unique metadata id.
    pub(crate) fn next() -> Tag {
        let id = COUNTER.fetch_add(1, Ordering::AcqRel);
        if id > Self::METADATA_ID_MASK {
            panic!("figment: out of unique tag IDs");
        }

        Tag::new(id, ProfileTag::Default)
    }

    pub(crate) fn metadata_id(self) -> u64 {
        (self.0 & Self::METADATA_ID_MASK) >> Self::METADATA_ID_SHIFT
    }

    pub(crate) fn profile_tag(self) -> ProfileTag {
        let bits = (self.0 & Self::PROFILE_TAG_MASK) >> Self::PROFILE_TAG_SHIFT;
        (bits as u8).into()
    }

    pub(crate) fn for_profile(self, profile: &crate::Profile) -> Self {
        Tag::new(self.metadata_id(), profile.into())
    }

    /// Returns `true` if `self` is `Tag::Default`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::value::Tag;
    ///
    /// assert!(Tag::Default.is_default());
    /// ```
    pub const fn is_default(self) -> bool {
        self.0 == Tag::Default.0
    }

    /// Returns the profile `self` refers to if it is either `Profile::Default`
    /// or `Profile::Custom`; otherwise returns `None`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use figment::Profile;
    /// use figment::value::Tag;
    ///
    /// assert_eq!(Tag::Default.profile(), Some(Profile::Default));
    /// ```
    pub fn profile(self) -> Option<Profile> {
        self.profile_tag().into()
    }
}

impl Default for Tag {
    fn default() -> Self {
        Tag::Default
    }
}

impl PartialEq for Tag {
    fn eq(&self, other: &Self) -> bool {
        self.metadata_id() == other.metadata_id()
    }
}

impl Eq for Tag {  }

impl PartialOrd for Tag {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.metadata_id().partial_cmp(&other.metadata_id())
    }
}

impl Ord for Tag {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.metadata_id().cmp(&other.metadata_id())
    }
}

impl std::hash::Hash for Tag {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_u64(self.metadata_id())
    }
}

impl From<Tag> for crate::value::Value {
    fn from(tag: Tag) -> Self {
        crate::value::Value::from(tag.0)
    }
}

impl<'de> de::Deserialize<'de> for Tag {
    fn deserialize<D>(deserializer: D) -> Result<Tag, D::Error>
        where D: de::Deserializer<'de>
    {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = Tag;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a 64-bit metadata id integer")
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(Tag(v))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

impl ser::Serialize for Tag {
    fn serialize<S: ser::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_u64(self.0)
    }
}

impl fmt::Debug for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            t if t.is_default() => write!(f, "Tag::Default"),
            _ => write!(f, "Tag({:?}, {})", self.profile_tag(), self.metadata_id())
        }
    }
}
