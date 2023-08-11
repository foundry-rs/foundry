use crate::{Config, Warning, DEPRECATIONS};
use figment::{
    value::{Dict, Map, Value},
    Error, Figment, Metadata, Profile, Provider,
};

/// Remappings provider
pub mod remappings;

/// Generate warnings for unknown sections and deprecated keys
pub struct WarningsProvider<P> {
    provider: P,
    profile: Profile,
    old_warnings: Result<Vec<Warning>, Error>,
}

impl<P> WarningsProvider<P> {
    const WARNINGS_KEY: &'static str = "__warnings";

    /// Creates a new warnings provider.
    pub fn new(
        provider: P,
        profile: impl Into<Profile>,
        old_warnings: Result<Vec<Warning>, Error>,
    ) -> Self {
        Self { provider, profile: profile.into(), old_warnings }
    }

    /// Creates a new figment warnings provider.
    pub fn for_figment(provider: P, figment: &Figment) -> Self {
        let old_warnings = {
            let warnings_res = figment.extract_inner(Self::WARNINGS_KEY);
            if warnings_res.as_ref().err().map(|err| err.missing()).unwrap_or(false) {
                Ok(vec![])
            } else {
                warnings_res
            }
        };
        Self::new(provider, figment.profile().clone(), old_warnings)
    }
}

impl<P: Provider> WarningsProvider<P> {
    /// Collects all warnings.
    pub fn collect_warnings(&self) -> Result<Vec<Warning>, Error> {
        let mut out = self.old_warnings.clone()?;
        // add warning for unknown sections
        out.extend(
            self.provider
                .data()
                .unwrap_or_default()
                .keys()
                .filter(|k| {
                    k != &Config::PROFILE_SECTION &&
                        !Config::STANDALONE_SECTIONS.iter().any(|s| s == k)
                })
                .map(|unknown_section| {
                    let source = self.provider.metadata().source.map(|s| s.to_string());
                    Warning::UnknownSection { unknown_section: unknown_section.clone(), source }
                }),
        );
        // add warning for deprecated keys
        out.extend(
            self.provider
                .data()
                .unwrap_or_default()
                .iter()
                .flat_map(|(profile, dict)| dict.keys().map(move |key| format!("{profile}.{key}")))
                .filter(|k| DEPRECATIONS.contains_key(k))
                .map(|deprecated_key| Warning::DeprecatedKey {
                    old: deprecated_key.clone(),
                    new: DEPRECATIONS.get(&deprecated_key).unwrap().to_string(),
                }),
        );
        Ok(out)
    }
}

impl<P: Provider> Provider for WarningsProvider<P> {
    fn metadata(&self) -> Metadata {
        if let Some(source) = self.provider.metadata().source {
            Metadata::from("Warnings", source)
        } else {
            Metadata::named("Warnings")
        }
    }
    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        Ok(Map::from([(
            self.profile.clone(),
            Dict::from([(
                Self::WARNINGS_KEY.to_string(),
                Value::serialize(self.collect_warnings()?)?,
            )]),
        )]))
    }
    fn profile(&self) -> Option<Profile> {
        Some(self.profile.clone())
    }
}

/// Extracts the profile from the `profile` key and sets unset values according to the fallback
/// provider
pub struct FallbackProfileProvider<P> {
    provider: P,
    profile: Profile,
    fallback: Profile,
}

impl<P> FallbackProfileProvider<P> {
    /// Creates a new fallback profile provider.
    pub fn new(provider: P, profile: impl Into<Profile>, fallback: impl Into<Profile>) -> Self {
        FallbackProfileProvider { provider, profile: profile.into(), fallback: fallback.into() }
    }
}

impl<P: Provider> Provider for FallbackProfileProvider<P> {
    fn metadata(&self) -> Metadata {
        self.provider.metadata()
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        if let Some(fallback) = self.provider.data()?.get(&self.fallback) {
            let mut inner = self.provider.data()?.remove(&self.profile).unwrap_or_default();
            for (k, v) in fallback.iter() {
                if !inner.contains_key(k) {
                    inner.insert(k.to_owned(), v.clone());
                }
            }
            Ok(self.profile.collect(inner))
        } else {
            self.provider.data()
        }
    }

    fn profile(&self) -> Option<Profile> {
        Some(self.profile.clone())
    }
}
