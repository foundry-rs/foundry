use super::VyperLanguage;
use crate::{
    artifacts::vyper::{VyperInput, VyperSettings},
    compilers::CompilerInput,
};
use foundry_compilers_artifacts::sources::{Source, Sources};
use semver::Version;
use serde::Serialize;
use std::{borrow::Cow, path::Path};

#[derive(Clone, Debug, Serialize)]
pub struct VyperVersionedInput {
    #[serde(flatten)]
    pub input: VyperInput,
    #[serde(skip)]
    pub version: Version,
}

impl CompilerInput for VyperVersionedInput {
    type Settings = VyperSettings;
    type Language = VyperLanguage;

    fn build(
        sources: Sources,
        settings: Self::Settings,
        _language: Self::Language,
        version: Version,
    ) -> Self {
        Self { input: VyperInput::new(sources, settings, &version), version }
    }

    fn compiler_name(&self) -> Cow<'static, str> {
        "Vyper".into()
    }

    fn strip_prefix(&mut self, base: &Path) {
        self.input.strip_prefix(base);
    }

    fn language(&self) -> Self::Language {
        VyperLanguage
    }

    fn version(&self) -> &Version {
        &self.version
    }

    fn sources(&self) -> impl Iterator<Item = (&Path, &Source)> {
        self.input
            .sources
            .iter()
            .chain(self.input.interfaces.iter())
            .map(|(path, source)| (path.as_path(), source))
    }
}
