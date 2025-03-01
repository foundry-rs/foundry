use crate::eip4844::trusted_setup_points::{G1_POINTS, G2_POINTS};
use alloc::sync::Arc;
use core::hash::{Hash, Hasher};

// Re-export for convenience
pub use c_kzg::KzgSettings;

/// KZG settings.
#[derive(Clone, Debug, Default, Eq)]
pub enum EnvKzgSettings {
    /// Default mainnet trusted setup.
    #[default]
    Default,
    /// Custom trusted setup.
    Custom(Arc<KzgSettings>),
}

// Implement PartialEq and Hash manually because `c_kzg::KzgSettings` does not implement them.
impl PartialEq for EnvKzgSettings {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Default, Self::Default) => true,
            (Self::Custom(a), Self::Custom(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Hash for EnvKzgSettings {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::Default => {}
            Self::Custom(settings) => Arc::as_ptr(settings).hash(state),
        }
    }
}

impl EnvKzgSettings {
    /// Returns the KZG settings.
    ///
    /// This will initialize the default settings if it is not already loaded.
    #[inline]
    pub fn get(&self) -> &KzgSettings {
        match self {
            Self::Default => {
                let load = || {
                    KzgSettings::load_trusted_setup(&G1_POINTS.0, &G2_POINTS.0)
                        .expect("failed to load default trusted setup")
                };
                #[cfg(feature = "std")]
                {
                    use once_cell as _;
                    use std::sync::OnceLock;
                    static DEFAULT: OnceLock<KzgSettings> = OnceLock::new();
                    DEFAULT.get_or_init(load)
                }
                #[cfg(not(feature = "std"))]
                {
                    use once_cell::race::OnceBox;
                    static DEFAULT: OnceBox<KzgSettings> = OnceBox::new();
                    DEFAULT.get_or_init(|| alloc::boxed::Box::new(load()))
                }
            }
            Self::Custom(settings) => settings,
        }
    }

    /// Load custom KZG settings from a trusted setup file.
    #[cfg(feature = "std")]
    pub fn load_from_trusted_setup_file(
        trusted_setup_file: &std::path::Path,
    ) -> Result<Self, c_kzg::Error> {
        let settings = KzgSettings::load_trusted_setup_file(trusted_setup_file)?;
        Ok(Self::Custom(Arc::new(settings)))
    }
}
