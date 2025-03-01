use crate::util::prelude::*;
use darling::FromMeta;
use std::fmt;
use std::ops::Deref;

/// A type that stores the attribute key path information along with the parsed value.
/// It is useful for error reporting. For example, if some key was unexpected, it's
/// possible to point to the key's span in the error instead of the attribute's value.
#[derive(Clone)]
pub(crate) struct SpannedKey<T> {
    pub(crate) key: syn::Ident,
    pub(crate) value: T,
}

impl<T> SpannedKey<T> {
    pub(crate) fn new(path: &syn::Path, value: T) -> Result<Self> {
        Ok(Self {
            key: path.require_ident()?.clone(),
            value,
        })
    }

    pub(crate) fn into_value(self) -> T {
        self.value
    }

    pub(crate) fn key(&self) -> &syn::Ident {
        &self.key
    }

    pub(crate) fn with_value<U>(self, value: U) -> SpannedKey<U> {
        SpannedKey {
            value,
            key: self.key,
        }
    }

    pub(crate) fn map_value<U>(self, map: impl FnOnce(T) -> U) -> SpannedKey<U> {
        SpannedKey {
            value: map(self.value),
            key: self.key,
        }
    }
}

impl<T: FromMeta> FromMeta for SpannedKey<T> {
    fn from_meta(meta: &syn::Meta) -> Result<Self> {
        let value = T::from_meta(meta)?;
        Self::new(meta.path(), value)
    }
}

impl<T: fmt::Debug> fmt::Debug for SpannedKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.value, f)
    }
}

impl<T> Deref for SpannedKey<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}
