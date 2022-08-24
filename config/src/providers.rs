use crate::{Config, Warning};
use figment::{
    value::{Dict, Map, Value},
    Error, Figment, Metadata, Profile, Provider,
};

/// Generate warnings for unknown sections
pub struct WarningsProvider<P> {
    provider: P,
    profile: Profile,
    old_warnings: Result<Vec<Warning>, Error>,
}

impl<P> WarningsProvider<P> {
    const WARNINGS_KEY: &'static str = "__warnings";

    pub fn new(
        provider: P,
        profile: impl Into<Profile>,
        old_warnings: Result<Vec<Warning>, Error>,
    ) -> Self {
        Self { provider, profile: profile.into(), old_warnings }
    }

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
    pub fn collect_warnings(&self) -> Result<Vec<Warning>, Error> {
        let mut out = self.old_warnings.clone()?;
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
