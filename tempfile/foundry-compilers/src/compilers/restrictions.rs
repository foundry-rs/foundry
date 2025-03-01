use std::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use semver::VersionReq;

/// Abstraction over set of restrictions for given [`crate::Compiler::Settings`].
pub trait CompilerSettingsRestrictions: Copy + Debug + Sync + Send + Clone + Default {
    /// Combines this restriction with another one. Returns `None` if restrictions are incompatible.
    #[must_use]
    fn merge(self, other: Self) -> Option<Self>;
}

/// Combines [CompilerSettingsRestrictions] with a restrictions on compiler versions for a given
/// source file.
#[derive(Debug, Clone, Default)]
pub struct RestrictionsWithVersion<T> {
    pub version: Option<VersionReq>,
    pub restrictions: T,
}

impl<T: CompilerSettingsRestrictions> RestrictionsWithVersion<T> {
    /// Tries to merge the given restrictions with the other [`RestrictionsWithVersion`]. Returns
    /// `None` if restrictions are incompatible.
    pub fn merge(mut self, other: Self) -> Option<Self> {
        if let Some(version) = other.version {
            if let Some(self_version) = self.version.as_mut() {
                self_version.comparators.extend(version.comparators);
            } else {
                self.version = Some(version);
            }
        }
        self.restrictions = self.restrictions.merge(other.restrictions)?;
        Some(self)
    }
}

impl<T> Deref for RestrictionsWithVersion<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.restrictions
    }
}

impl<T> DerefMut for RestrictionsWithVersion<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.restrictions
    }
}
