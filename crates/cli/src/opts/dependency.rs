//! CLI dependency parsing

use eyre::Result;
use regex::Regex;
use std::{str::FromStr, sync::LazyLock};

static GH_REPO_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\w-]+/[\w.-]+").unwrap());

/// Git repo prefix regex
pub static GH_REPO_PREFIX_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"((git@)|(git\+https://)|(https://)|(org-([A-Za-z0-9-])+@))?(?P<brand>[A-Za-z0-9-]+)\.(?P<tld>[A-Za-z0-9-]+)(/|:)")
        .unwrap()
});

const GITHUB: &str = "github.com";
const VERSION_SEPARATOR: char = '@';
const ALIAS_SEPARATOR: char = '=';

/// Commonly used aliases for solidity repos,
///
/// These will be autocorrected when used in place of the `org`
const COMMON_ORG_ALIASES: &[(&str, &str); 2] =
    &[("@openzeppelin", "openzeppelin"), ("@aave", "aave")];

/// A git dependency which will be installed as a submodule
///
/// A dependency can be provided as a raw URL, or as a path to a Github repository
/// e.g. `org-name/repo-name`
///
/// Providing a ref can be done in the following 3 ways:
/// * branch: master
/// * tag: v0.1.1
/// * commit: 8e8128
///
/// Non Github URLs must be provided with an https:// prefix.
/// Adding dependencies as local paths is not supported yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dependency {
    /// The name of the dependency
    pub name: String,
    /// The url to the git repository corresponding to the dependency
    pub url: Option<String>,
    /// Optional tag corresponding to a Git SHA, tag, or branch.
    pub tag: Option<String>,
    /// Optional alias of the dependency
    pub alias: Option<String>,
}

impl FromStr for Dependency {
    type Err = eyre::Error;
    fn from_str(dependency: &str) -> Result<Self, Self::Err> {
        // everything before "=" should be considered the alias
        let (mut alias, dependency) = if let Some(split) = dependency.split_once(ALIAS_SEPARATOR) {
            (Some(String::from(split.0)), split.1.to_string())
        } else {
            let mut dependency = dependency.to_string();
            // this will autocorrect wrong conventional aliases for tag, but only autocorrect if
            // it's not used as alias
            for (alias, real_org) in COMMON_ORG_ALIASES.iter() {
                if dependency.starts_with(alias) {
                    dependency = dependency.replacen(alias, real_org, 1);
                    break
                }
            }

            (None, dependency)
        };

        let dependency = dependency.as_str();

        let url_with_version = if let Some(captures) = GH_REPO_PREFIX_REGEX.captures(dependency) {
            let brand = captures.name("brand").unwrap().as_str();
            let tld = captures.name("tld").unwrap().as_str();
            let project = GH_REPO_PREFIX_REGEX.replace(dependency, "");
            Some(format!("https://{brand}.{tld}/{}", project.trim_end_matches(".git")))
        } else {
            // If we don't have a URL and we don't have a valid
            // GitHub repository name, then we assume this is the alias.
            //
            // This is to allow for conveniently removing aliased dependencies
            // using `forge remove <alias>`
            if GH_REPO_REGEX.is_match(dependency) {
                Some(format!("https://{GITHUB}/{dependency}"))
            } else {
                alias = Some(dependency.to_string());
                None
            }
        };

        // everything after the last "@" should be considered the version if there are no path
        // segments
        let (url, name, tag) = if let Some(url_with_version) = url_with_version {
            // `@`s are actually valid github project name chars but we assume this is unlikely and
            // treat everything after the last `@` as the version tag there's still the
            // case that the user tries to use `@<org>/<project>`, so we need to check that the
            // `tag` does not contain a slash
            let mut split = url_with_version.rsplit(VERSION_SEPARATOR);

            let mut tag = None;
            let mut url = url_with_version.as_str();

            let maybe_tag = split.next().unwrap();
            if let Some(actual_url) = split.next() {
                if !maybe_tag.contains('/') {
                    tag = Some(maybe_tag.to_string());
                    url = actual_url;
                }
            }

            let url = url.to_string();
            let name = url
                .split('/')
                .last()
                .ok_or_else(|| eyre::eyre!("no dependency name found"))?
                .to_string();

            (Some(url), Some(name), tag)
        } else {
            (None, None, None)
        };

        Ok(Self { name: name.or_else(|| alias.clone()).unwrap(), url, tag, alias })
    }
}

impl Dependency {
    /// Returns the name of the dependency, prioritizing the alias if it exists.
    pub fn name(&self) -> &str {
        self.alias.as_deref().unwrap_or(self.name.as_str())
    }

    /// Returns the URL of the dependency if it exists, or an error if not.
    pub fn require_url(&self) -> Result<&str> {
        self.url.as_deref().ok_or_else(|| eyre::eyre!("dependency {} has no url", self.name()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_compilers::info::ContractInfo;

    #[test]
    fn parses_dependencies() {
        [
            ("gakonst/lootloose", "https://github.com/gakonst/lootloose", None, None),
            ("github.com/gakonst/lootloose", "https://github.com/gakonst/lootloose", None, None),
            (
                "https://github.com/gakonst/lootloose",
                "https://github.com/gakonst/lootloose",
                None,
                None,
            ),
            (
                "git+https://github.com/gakonst/lootloose",
                "https://github.com/gakonst/lootloose",
                None,
                None,
            ),
            (
                "git@github.com:gakonst/lootloose@v1",
                "https://github.com/gakonst/lootloose",
                Some("v1"),
                None,
            ),
            (
                "git@github.com:gakonst/lootloose",
                "https://github.com/gakonst/lootloose",
                None,
                None,
            ),
            (
                "https://gitlab.com/gakonst/lootloose",
                "https://gitlab.com/gakonst/lootloose",
                None,
                None,
            ),
            (
                "https://github.xyz/gakonst/lootloose",
                "https://github.xyz/gakonst/lootloose",
                None,
                None,
            ),
            (
                "gakonst/lootloose@0.1.0",
                "https://github.com/gakonst/lootloose",
                Some("0.1.0"),
                None,
            ),
            (
                "gakonst/lootloose@develop",
                "https://github.com/gakonst/lootloose",
                Some("develop"),
                None,
            ),
            (
                "gakonst/lootloose@98369d0edc900c71d0ec33a01dfba1d92111deed",
                "https://github.com/gakonst/lootloose",
                Some("98369d0edc900c71d0ec33a01dfba1d92111deed"),
                None,
            ),
            ("loot=gakonst/lootloose", "https://github.com/gakonst/lootloose", None, Some("loot")),
            (
                "loot=github.com/gakonst/lootloose",
                "https://github.com/gakonst/lootloose",
                None,
                Some("loot"),
            ),
            (
                "loot=https://github.com/gakonst/lootloose",
                "https://github.com/gakonst/lootloose",
                None,
                Some("loot"),
            ),
            (
                "loot=git+https://github.com/gakonst/lootloose",
                "https://github.com/gakonst/lootloose",
                None,
                Some("loot"),
            ),
            (
                "loot=git@github.com:gakonst/lootloose@v1",
                "https://github.com/gakonst/lootloose",
                Some("v1"),
                Some("loot"),
            ),
        ]
        .iter()
        .for_each(|(input, expected_path, expected_tag, expected_alias)| {
            let dep = Dependency::from_str(input).unwrap();
            assert_eq!(dep.url, Some(expected_path.to_string()));
            assert_eq!(dep.tag, expected_tag.map(ToString::to_string));
            assert_eq!(dep.name, "lootloose");
            assert_eq!(dep.alias, expected_alias.map(ToString::to_string));
        });
    }

    #[test]
    fn can_parse_alias_only() {
        let dep = Dependency::from_str("foo").unwrap();
        assert_eq!(dep.name, "foo");
        assert_eq!(dep.url, None);
        assert_eq!(dep.tag, None);
        assert_eq!(dep.alias, Some("foo".to_string()));
    }

    #[test]
    fn test_invalid_github_repo_dependency() {
        let dep = Dependency::from_str("solmate").unwrap();
        assert_eq!(dep.url, None);
    }

    #[test]
    fn parses_contract_info() {
        [
            (
                "src/contracts/Contracts.sol:Contract",
                Some("src/contracts/Contracts.sol"),
                "Contract",
            ),
            ("Contract", None, "Contract"),
        ]
        .iter()
        .for_each(|(input, expected_path, expected_name)| {
            let contract = ContractInfo::from_str(input).unwrap();
            assert_eq!(contract.path, expected_path.map(ToString::to_string));
            assert_eq!(contract.name, expected_name.to_string());
        });
    }

    #[test]
    fn contract_info_should_reject_without_name() {
        ["src/contracts/", "src/contracts/Contracts.sol"].iter().for_each(|input| {
            let contract = ContractInfo::from_str(input);
            assert!(contract.is_err())
        });
    }

    #[test]
    fn can_parse_oz_dep() {
        let dep = Dependency::from_str("@openzeppelin/contracts-upgradeable").unwrap();
        assert_eq!(dep.name, "contracts-upgradeable");
        assert_eq!(
            dep.url,
            Some("https://github.com/openzeppelin/contracts-upgradeable".to_string())
        );
        assert_eq!(dep.tag, None);
        assert_eq!(dep.alias, None);
    }

    #[test]
    fn can_parse_oz_dep_tag() {
        let dep = Dependency::from_str("@openzeppelin/contracts-upgradeable@v1").unwrap();
        assert_eq!(dep.name, "contracts-upgradeable");
        assert_eq!(
            dep.url,
            Some("https://github.com/openzeppelin/contracts-upgradeable".to_string())
        );
        assert_eq!(dep.tag, Some("v1".to_string()));
        assert_eq!(dep.alias, None);
    }

    #[test]
    fn can_parse_oz_with_tag() {
        let dep = Dependency::from_str("OpenZeppelin/openzeppelin-contracts@v4.7.0").unwrap();
        assert_eq!(dep.name, "openzeppelin-contracts");
        assert_eq!(
            dep.url,
            Some("https://github.com/OpenZeppelin/openzeppelin-contracts".to_string())
        );
        assert_eq!(dep.tag, Some("v4.7.0".to_string()));
        assert_eq!(dep.alias, None);

        let dep = Dependency::from_str("OpenZeppelin/openzeppelin-contracts@4.7.0").unwrap();
        assert_eq!(dep.name, "openzeppelin-contracts");
        assert_eq!(
            dep.url,
            Some("https://github.com/OpenZeppelin/openzeppelin-contracts".to_string())
        );
        assert_eq!(dep.tag, Some("4.7.0".to_string()));
        assert_eq!(dep.alias, None);
    }

    // <https://github.com/foundry-rs/foundry/pull/3130>
    #[test]
    fn can_parse_oz_with_alias() {
        let dep =
            Dependency::from_str("@openzeppelin=OpenZeppelin/openzeppelin-contracts").unwrap();
        assert_eq!(dep.name, "openzeppelin-contracts");
        assert_eq!(dep.alias, Some("@openzeppelin".to_string()));
        assert_eq!(
            dep.url,
            Some("https://github.com/OpenZeppelin/openzeppelin-contracts".to_string())
        );
    }

    #[test]
    fn can_parse_aave() {
        let dep = Dependency::from_str("@aave/aave-v3-core").unwrap();
        assert_eq!(dep.name, "aave-v3-core");
        assert_eq!(dep.url, Some("https://github.com/aave/aave-v3-core".to_string()));
    }

    #[test]
    fn can_parse_aave_with_alias() {
        let dep = Dependency::from_str("@aave=aave/aave-v3-core").unwrap();
        assert_eq!(dep.name, "aave-v3-core");
        assert_eq!(dep.alias, Some("@aave".to_string()));
        assert_eq!(dep.url, Some("https://github.com/aave/aave-v3-core".to_string()));
    }

    #[test]
    fn can_parse_org_ssh_url() {
        let org_url = "org-git12345678@github.com:my-org/my-repo.git";
        assert!(GH_REPO_PREFIX_REGEX.is_match(org_url));
    }

    #[test]
    fn can_parse_org_shh_url_dependency() {
        let dep: Dependency = "org-git12345678@github.com:my-org/my-repo.git".parse().unwrap();
        assert_eq!(dep.url.unwrap(), "https://github.com/my-org/my-repo");
    }
}
