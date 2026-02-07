use crate::{Config, DEPRECATIONS, Warning};
use figment::{
    Error, Figment, Metadata, Profile, Provider,
    value::{Dict, Map, Value},
};
use heck::ToSnakeCase;
use std::collections::{BTreeMap, BTreeSet};

/// Allowed keys for CompilationRestrictions.
const COMPILATION_RESTRICTIONS_KEYS: &[&str] = &[
    "paths",
    "version",
    "via_ir",
    "bytecode_hash",
    "min_optimizer_runs",
    "optimizer_runs",
    "max_optimizer_runs",
    "min_evm_version",
    "evm_version",
    "max_evm_version",
];

/// Allowed keys for SettingsOverrides.
const SETTINGS_OVERRIDES_KEYS: &[&str] =
    &["name", "via_ir", "evm_version", "optimizer", "optimizer_runs", "bytecode_hash"];

/// Allowed keys for VyperConfig.
/// Required because VyperConfig uses `skip_serializing_if = "Option::is_none"` on all fields,
/// causing the default serialization to produce an empty dict.
const VYPER_KEYS: &[&str] = &["optimize", "path", "experimental_codegen"];

/// Reserved keys that should not trigger unknown key warnings.
const RESERVED_KEYS: &[&str] = &["extends"];

/// Keys kept for backward compatibility that should not trigger unknown key warnings.
const BACKWARD_COMPATIBLE_KEYS: &[&str] = &["solc_version"];

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
        out.extend(data.keys().filter(|k| !Config::is_standalone_section(k.as_str())).map(
            |unknown_section| {
                let source = self.provider.metadata().source.map(|s| s.to_string());
                Warning::UnknownSection { unknown_section: unknown_section.clone(), source }
            },
        ));

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
                .clone()
                .filter_map(|dict| dict.get(self.profile.as_str().as_str()))
                .filter_map(Value::as_dict)
                .flat_map(BTreeMap::keys)
                .filter_map(deprecated_key_warning),
        );

        // Add warning for unknown keys within profiles (root keys only here).
        if let Ok(default_map) = figment::providers::Serialized::defaults(&Config::default()).data()
            && let Some(default_dict) = default_map.get(&Config::DEFAULT_PROFILE)
        {
            let allowed_keys: BTreeSet<String> = default_dict.keys().cloned().collect();
            for profile_map in profiles.clone() {
                for (profile, value) in profile_map {
                    let Some(profile_dict) = value.as_dict() else {
                        continue;
                    };

                    let source = self
                        .provider
                        .metadata()
                        .source
                        .map(|s| s.to_string())
                        .unwrap_or(Config::FILE_NAME.to_string());
                    for key in profile_dict.keys() {
                        let is_not_deprecated =
                            !DEPRECATIONS.iter().any(|(deprecated_key, _)| *deprecated_key == key);
                        let is_not_allowed = !allowed_keys.contains(key)
                            && !allowed_keys.contains(&key.to_snake_case());
                        let is_not_reserved =
                            !RESERVED_KEYS.contains(&key.as_str()) && key != Self::WARNINGS_KEY;
                        let is_not_backward_compatible =
                            !BACKWARD_COMPATIBLE_KEYS.contains(&key.as_str());

                        if is_not_deprecated
                            && is_not_allowed
                            && is_not_reserved
                            && is_not_backward_compatible
                        {
                            out.push(Warning::UnknownKey {
                                key: key.clone(),
                                profile: profile.clone(),
                                source: source.clone(),
                            });
                        }
                    }

                    // Add warning for unknown keys in nested sections within profiles.
                    self.collect_nested_section_warnings(
                        profile_dict,
                        default_dict,
                        &source,
                        &mut out,
                    );
                }
            }

            // Add warning for unknown keys in standalone sections.
            self.collect_standalone_section_warnings(&data, default_dict, &mut out);
        }

        Ok(out)
    }

    /// Collects warnings for unknown keys in standalone sections like `[lint]`, `[fmt]`, etc.
    fn collect_standalone_section_warnings(
        &self,
        data: &Map<Profile, Dict>,
        default_dict: &Dict,
        out: &mut Vec<Warning>,
    ) {
        let source = self
            .provider
            .metadata()
            .source
            .map(|s| s.to_string())
            .unwrap_or(Config::FILE_NAME.to_string());

        for section_name in Config::STANDALONE_SECTIONS {
            // Get the section from the parsed data
            let section_profile = Profile::new(section_name);
            let Some(section_dict) = data.get(&section_profile) else {
                continue;
            };

            // Get allowed keys for this section from the default config
            // Special case for vyper: VyperConfig uses skip_serializing_if on all Option fields,
            // so the default serialization produces an empty dict. Use explicit keys instead.
            let allowed_keys: BTreeSet<String> = if *section_name == "vyper" {
                VYPER_KEYS.iter().map(|s| s.to_string()).collect()
            } else {
                let Some(default_section_value) = default_dict.get(*section_name) else {
                    continue;
                };
                let Some(default_section_dict) = default_section_value.as_dict() else {
                    continue;
                };
                default_section_dict.keys().cloned().collect()
            };

            for key in section_dict.keys() {
                let is_not_allowed =
                    !allowed_keys.contains(key) && !allowed_keys.contains(&key.to_snake_case());
                if is_not_allowed {
                    out.push(Warning::UnknownSectionKey {
                        key: key.clone(),
                        section: section_name.to_string(),
                        source: source.clone(),
                    });
                }
            }
        }
    }

    /// Collects warnings for unknown keys in nested sections within profiles,
    /// like `compilation_restrictions`.
    fn collect_nested_section_warnings(
        &self,
        profile_dict: &Dict,
        default_dict: &Dict,
        source: &str,
        out: &mut Vec<Warning>,
    ) {
        // Check nested sections that are dicts (like `lint`, `fmt` when defined in profile)
        for (key, value) in profile_dict {
            let Some(nested_dict) = value.as_dict() else {
                // Also check arrays of dicts (like `compilation_restrictions`)
                if let Some(arr) = value.as_array() {
                    // Get allowed keys for known array item types
                    let allowed_keys = Self::get_array_item_allowed_keys(key);

                    if allowed_keys.is_empty() {
                        continue;
                    }

                    for item in arr {
                        let Some(item_dict) = item.as_dict() else {
                            continue;
                        };
                        for item_key in item_dict.keys() {
                            let is_not_allowed = !allowed_keys.contains(item_key)
                                && !allowed_keys.contains(&item_key.to_snake_case());
                            if is_not_allowed {
                                out.push(Warning::UnknownSectionKey {
                                    key: item_key.clone(),
                                    section: key.clone(),
                                    source: source.to_string(),
                                });
                            }
                        }
                    }
                }
                continue;
            };

            // Get allowed keys from the default config for this nested section
            // Special case for vyper: VyperConfig uses skip_serializing_if on all Option fields,
            // so the default serialization produces an empty dict. Use explicit keys instead.
            let allowed_keys: BTreeSet<String> = if key == "vyper" {
                VYPER_KEYS.iter().map(|s| s.to_string()).collect()
            } else {
                let Some(default_value) = default_dict.get(key) else {
                    continue;
                };
                let Some(default_nested_dict) = default_value.as_dict() else {
                    continue;
                };
                default_nested_dict.keys().cloned().collect()
            };

            for nested_key in nested_dict.keys() {
                let is_not_allowed = !allowed_keys.contains(nested_key)
                    && !allowed_keys.contains(&nested_key.to_snake_case());
                if is_not_allowed {
                    out.push(Warning::UnknownSectionKey {
                        key: nested_key.clone(),
                        section: key.clone(),
                        source: source.to_string(),
                    });
                }
            }
        }
    }

    /// Returns the allowed keys for array item types based on the section name.
    fn get_array_item_allowed_keys(section_name: &str) -> BTreeSet<String> {
        match section_name {
            "compilation_restrictions" => {
                COMPILATION_RESTRICTIONS_KEYS.iter().map(|s| s.to_string()).collect()
            }
            "additional_compiler_profiles" => {
                SETTINGS_OVERRIDES_KEYS.iter().map(|s| s.to_string()).collect()
            }
            _ => BTreeSet::new(),
        }
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
