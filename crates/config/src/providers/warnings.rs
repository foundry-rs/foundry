use crate::{Config, DEPRECATIONS, Warning};
use figment::{
    Error, Figment, Metadata, Profile, Provider,
    value::{Dict, Map, Value},
};
use std::collections::{BTreeMap, BTreeSet};

const LABELS_KEY: &str = "labels";
const TRACING_LABELS_KEY: &str = "tracing.labels";

/// Generates warnings for supported deprecated configuration.
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
        let data = self.provider.data()?;

        let mut out = self.old_warnings.clone()?;
        // Non-standalone top-level tables are legacy implicit profiles. Preserve the migration
        // warning while strict validation handles their contents.
        out.extend(data.keys().filter(|key| !Config::is_standalone_section(key.as_str())).map(
            |unknown_section| Warning::UnknownSection {
                unknown_section: unknown_section.clone(),
                source: self.provider.metadata().source.map(|source| source.to_string()),
            },
        ));

        // Add warning for deprecated keys.
        let deprecated_key_warning = |key| {
            DEPRECATIONS.iter().find_map(|(deprecated_key, new_value)| {
                (key == *deprecated_key).then(|| Warning::DeprecatedKey {
                    old: deprecated_key.to_string(),
                    new: new_value.to_string(),
                })
            })
        };
        let profiles = data
            .iter()
            .filter(|(profile, _)| **profile == Config::PROFILE_SECTION)
            .map(|(_, dict)| dict);

        let deprecated_profile_keys = profiles
            .clone()
            .flat_map(|dict| {
                dict.keys().chain(dict.values().filter_map(Value::as_dict).flat_map(BTreeMap::keys))
            })
            .collect::<BTreeSet<_>>();
        out.extend(deprecated_profile_keys.into_iter().filter_map(deprecated_key_warning));
        self.collect_deprecated_label_warnings(&data, profiles.clone(), &mut out);

        Ok(out)
    }

    fn collect_deprecated_label_warnings<'a>(
        &self,
        data: &Map<Profile, Dict>,
        profiles: impl Iterator<Item = &'a Dict>,
        out: &mut Vec<Warning>,
    ) {
        if data.contains_key(&Profile::new(LABELS_KEY)) {
            out.push(Self::deprecated_label_warning("[labels]", "[tracing.labels]"));
        }

        if profiles
            .flat_map(BTreeMap::values)
            .filter_map(Value::as_dict)
            .any(|dict| dict.contains_key(LABELS_KEY))
        {
            out.push(Self::deprecated_label_warning(LABELS_KEY, TRACING_LABELS_KEY));
        }

        if let Some(dict) = data.get(&self.profile)
            && dict.contains_key(LABELS_KEY)
        {
            out.push(Self::deprecated_label_warning(LABELS_KEY, TRACING_LABELS_KEY));
        }
    }

    fn deprecated_label_warning(old: &str, new: &str) -> Warning {
        Warning::DeprecatedKey { old: old.to_string(), new: new.to_string() }
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
