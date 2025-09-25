use crate::{Config, DEPRECATIONS, Warning};
use figment::{
    Error, Figment, Metadata, Profile, Provider,
    value::{Dict, Map, Value},
};
use heck::ToSnakeCase;
use std::collections::BTreeMap;

/// Generate warnings for unknown sections and deprecated keys
pub struct WarningsProvider<P> {
    provider: P,
    profile: Profile,
    old_warnings: Result<Vec<Warning>, Error>,
}

impl<P: Provider> WarningsProvider<P> {
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

    /// Collects all warnings.
    pub fn collect_warnings(&self) -> Result<Vec<Warning>, Error> {
        let data = self.provider.data().unwrap_or_default();

        let mut out = self.old_warnings.clone()?;

        // Add warning for unknown sections.
        out.extend(
            data.keys()
                .filter(|k| {
                    **k != Config::PROFILE_SECTION
                        && !Config::STANDALONE_SECTIONS.iter().any(|s| s == k)
                })
                .map(|unknown_section| {
                    let source = self.provider.metadata().source.map(|s| s.to_string());
                    Warning::UnknownSection { unknown_section: unknown_section.clone(), source }
                }),
        );

        // Add warning for deprecated keys.
        let deprecated_key_warning = |key| {
            DEPRECATIONS.iter().find_map(|(deprecated_key, new_value)| {
                if key == *deprecated_key {
                    Some(Warning::DeprecatedKey {
                        old: deprecated_key.to_string(),
                        new: new_value.to_string(),
                    })
                } else {
                    None
                }
            })
        };
        let profiles = data
            .iter()
            .filter(|(profile, _)| **profile == Config::PROFILE_SECTION)
            .map(|(_, dict)| dict);
        out.extend(profiles.clone().flat_map(BTreeMap::keys).filter_map(deprecated_key_warning));
        out.extend(
            profiles
                .filter_map(|dict| dict.get(self.profile.as_str().as_str()))
                .filter_map(Value::as_dict)
                .flat_map(BTreeMap::keys)
                .filter_map(deprecated_key_warning),
        );

        // Add warning for unknown keys within the active profile table (root keys only here).
        // Determine allowed top-level keys by serializing default Config to dict.
        // Note: this only checks keys under [profile.<active>] and does not dive into nested
        // subtables.
        if let Ok(default_map) = figment::providers::Serialized::defaults(&Config::default()).data()
            && let Some(default_dict) = default_map.get(&Config::DEFAULT_PROFILE)
        {
            let allowed_keys: std::collections::BTreeSet<String> =
                default_dict.keys().cloned().collect();

            let active_profile_dict = data
                .get(&figment::Profile::new(Config::PROFILE_SECTION))
                .and_then(|dict| dict.get(self.profile.as_str().as_str()))
                .and_then(Value::as_dict);

            if let Some(profile_dict) = active_profile_dict {
                for key in profile_dict.keys() {
                    if !allowed_keys.contains(key)
                        && !allowed_keys.contains(&key.to_snake_case())
                        && key != "extends"
                        && key != "__warnings"
                    {
                        let source = self.provider.metadata().source.map(|s| s.to_string());
                        out.push(Warning::UnknownKey {
                            key: key.clone(),
                            profile: Some(self.profile.to_string()),
                            source,
                        });
                    }
                }
            }
        }

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
        let warnings = self.collect_warnings()?;
        Ok(Map::from([(
            self.profile.clone(),
            Dict::from([(Self::WARNINGS_KEY.to_string(), Value::serialize(warnings)?)]),
        )]))
    }

    fn profile(&self) -> Option<Profile> {
        Some(self.profile.clone())
    }
}
