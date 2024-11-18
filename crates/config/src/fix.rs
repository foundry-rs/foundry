//! Helpers to automatically fix configuration warnings.

use crate::{Config, Warning};
use figment::providers::Env;
use std::{
    fs, io,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
};

/// A convenience wrapper around a TOML document and the path it was read from
struct TomlFile {
    doc: toml_edit::DocumentMut,
    path: PathBuf,
}

impl TomlFile {
    fn open(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let path = path.as_ref().to_owned();
        let doc = fs::read_to_string(&path)?.parse()?;
        Ok(Self { doc, path })
    }

    fn doc(&self) -> &toml_edit::DocumentMut {
        &self.doc
    }

    fn doc_mut(&mut self) -> &mut toml_edit::DocumentMut {
        &mut self.doc
    }

    fn path(&self) -> &Path {
        self.path.as_ref()
    }

    fn save(&self) -> io::Result<()> {
        fs::write(self.path(), self.doc().to_string())
    }
}

impl Deref for TomlFile {
    type Target = toml_edit::DocumentMut;
    fn deref(&self) -> &Self::Target {
        self.doc()
    }
}

impl DerefMut for TomlFile {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.doc_mut()
    }
}

/// The error emitted when failing to insert into a profile.
#[derive(Debug)]
struct InsertProfileError {
    pub message: String,
    pub value: toml_edit::Item,
}

impl std::fmt::Display for InsertProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for InsertProfileError {}

impl TomlFile {
    /// Insert a name as `[profile.name]`. Creating the `[profile]` table where necessary and
    /// throwing an error if there exists a conflict
    #[allow(clippy::result_large_err)]
    fn insert_profile(
        &mut self,
        profile_str: &str,
        value: toml_edit::Item,
    ) -> Result<(), InsertProfileError> {
        if !value.is_table_like() {
            return Err(InsertProfileError {
                message: format!("Expected [{profile_str}] to be a Table"),
                value,
            })
        }
        // get or create the profile section
        let profile_map = if let Some(map) = self.get_mut(Config::PROFILE_SECTION) {
            map
        } else {
            // insert profile section at the beginning of the map
            let mut profile_section = toml_edit::Table::new();
            profile_section.set_position(0);
            profile_section.set_implicit(true);
            self.insert(Config::PROFILE_SECTION, toml_edit::Item::Table(profile_section));
            self.get_mut(Config::PROFILE_SECTION).expect("exists per above")
        };
        // ensure the profile section is a table
        let profile_map = if let Some(table) = profile_map.as_table_like_mut() {
            table
        } else {
            return Err(InsertProfileError {
                message: format!("Expected [{}] to be a Table", Config::PROFILE_SECTION),
                value,
            })
        };
        // check the profile map for structure and existing keys
        if let Some(profile) = profile_map.get(profile_str) {
            if let Some(profile_table) = profile.as_table_like() {
                if !profile_table.is_empty() {
                    return Err(InsertProfileError {
                        message: format!(
                            "[{}.{}] already exists",
                            Config::PROFILE_SECTION,
                            profile_str
                        ),
                        value,
                    })
                }
            } else {
                return Err(InsertProfileError {
                    message: format!(
                        "Expected [{}.{}] to be a Table",
                        Config::PROFILE_SECTION,
                        profile_str
                    ),
                    value,
                })
            }
        }
        // insert the profile
        profile_map.insert(profile_str, value);
        Ok(())
    }
}

/// Making sure any implicit profile `[name]` becomes `[profile.name]` for the given file and
/// returns the implicit profiles and the result of editing them
fn fix_toml_non_strict_profiles(
    toml_file: &mut TomlFile,
) -> Vec<(String, Result<(), InsertProfileError>)> {
    let mut results = vec![];

    // get any non root level keys that need to be inserted into [profile]
    let profiles = toml_file
        .as_table()
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| {
            !(k == Config::PROFILE_SECTION || Config::STANDALONE_SECTIONS.contains(&k.as_str()))
        })
        .collect::<Vec<_>>();

    // remove each profile and insert into [profile] section
    for profile in profiles {
        if let Some(value) = toml_file.remove(&profile) {
            let res = toml_file.insert_profile(&profile, value);
            if let Err(err) = res.as_ref() {
                toml_file.insert(&profile, err.value.clone());
            }
            results.push((profile, res))
        }
    }
    results
}

/// Fix foundry.toml files. Making sure any implicit profile `[name]` becomes
/// `[profile.name]`. Return any warnings
pub fn fix_tomls() -> Vec<Warning> {
    let mut warnings = vec![];

    let tomls = {
        let mut tomls = vec![];
        if let Some(global_toml) = Config::foundry_dir_toml().filter(|p| p.exists()) {
            tomls.push(global_toml);
        }
        let local_toml = PathBuf::from(
            Env::var("FOUNDRY_CONFIG").unwrap_or_else(|| Config::FILE_NAME.to_string()),
        );
        if local_toml.exists() {
            tomls.push(local_toml);
        } else {
            warnings.push(Warning::NoLocalToml(local_toml));
        }
        tomls
    };

    for toml in tomls {
        let mut toml_file = match TomlFile::open(&toml) {
            Ok(toml_file) => toml_file,
            Err(err) => {
                warnings.push(Warning::CouldNotReadToml { path: toml, err: err.to_string() });
                continue
            }
        };

        let results = fix_toml_non_strict_profiles(&mut toml_file);
        let was_edited = results.iter().any(|(_, res)| res.is_ok());
        for (profile, err) in results
            .into_iter()
            .filter_map(|(profile, res)| res.err().map(|err| (profile, err.message)))
        {
            warnings.push(Warning::CouldNotFixProfile {
                path: toml_file.path().into(),
                profile,
                err,
            })
        }

        if was_edited {
            if let Err(err) = toml_file.save() {
                warnings.push(Warning::CouldNotWriteToml {
                    path: toml_file.path().into(),
                    err: err.to_string(),
                });
            }
        }
    }

    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use figment::Jail;
    use similar_asserts::assert_eq;

    macro_rules! fix_test {
        ($(#[$attr:meta])* $name:ident, $fun:expr) => {
            #[test]
            $(#[$attr])*
            fn $name() {
                Jail::expect_with(|jail| {
                    // setup home directory,
                    // **Note** this only has an effect on unix, as [`dirs_next::home_dir()`] on windows uses `FOLDERID_Profile`
                    jail.set_env("HOME", jail.directory().display().to_string());
                    std::fs::create_dir(jail.directory().join(".foundry")).unwrap();

                    // define function type to allow implicit params / return
                    let f: Box<dyn FnOnce(&mut Jail) -> Result<(), figment::Error>> = Box::new($fun);
                    f(jail)?;

                    Ok(())
                });
            }
        };
    }

    fix_test!(test_implicit_profile_name_changed, |jail| {
        jail.create_file(
            "foundry.toml",
            r#"
                [default]
                src = "src"
                # comment

                [other]
                src = "other-src"
            "#,
        )?;
        fix_tomls();
        assert_eq!(
            fs::read_to_string("foundry.toml").unwrap(),
            r#"
                [profile.default]
                src = "src"
                # comment

                [profile.other]
                src = "other-src"
            "#
        );
        Ok(())
    });

    fix_test!(test_leave_standalone_sections_alone, |jail| {
        jail.create_file(
            "foundry.toml",
            r#"
                [default]
                src = "src"

                [fmt]
                line_length = 100

                [rpc_endpoints]
                optimism = "https://example.com/"
            "#,
        )?;
        fix_tomls();
        assert_eq!(
            fs::read_to_string("foundry.toml").unwrap(),
            r#"
                [profile.default]
                src = "src"

                [fmt]
                line_length = 100

                [rpc_endpoints]
                optimism = "https://example.com/"
            "#
        );
        Ok(())
    });

    // mocking the `$HOME` has no effect on windows, see [`dirs_next::home_dir()`]
    fix_test!(
        #[cfg(not(windows))]
        test_global_toml_is_edited,
        |jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [other]
                src = "other-src"
            "#,
            )?;
            jail.create_file(
                ".foundry/foundry.toml",
                r#"
                [default]
                src = "src"
            "#,
            )?;
            fix_tomls();
            assert_eq!(
                fs::read_to_string("foundry.toml").unwrap(),
                r#"
                [profile.other]
                src = "other-src"
            "#
            );
            assert_eq!(
                fs::read_to_string(".foundry/foundry.toml").unwrap(),
                r#"
                [profile.default]
                src = "src"
            "#
            );
            Ok(())
        }
    );
}
