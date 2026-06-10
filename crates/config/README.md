# Configuration

Foundry's configuration system allows you to configure its tools the way _you_ want while also providing with a
sensible set of defaults.

## Profiles

Configurations can be arbitrarily namespaced with profiles. Foundry's default config is also named `default`, but you can
arbitrarily name and configure profiles as you like and set the `FOUNDRY_PROFILE` environment variable to the selected
profile's name. This results in foundry's tools (forge) preferring the values in the profile with the named that's set
in `FOUNDRY_PROFILE`. But all custom profiles inherit from the `default` profile.

## foundry.toml

Foundry's tools search for a `foundry.toml` or the filename in a `FOUNDRY_CONFIG` environment variable starting at the
current working directory. If it is not found, the parent directory, its parent directory, and so on are searched until
the file is found or the root is reached. But the typical location for the global `foundry.toml` would
be `~/.foundry/foundry.toml`, which is also checked. If the path set in `FOUNDRY_CONFIG` is absolute, no such search
takes place and the absolute path is used directly.

In `foundry.toml` you can define multiple profiles, therefore the file is assumed to be _nested_, so each top-level key
declares a profile and its values configure the profile.

The following is an example of what such a file might look like. This can also be obtained with `forge config`

```toml
## defaults for _all_ profiles
[profile.default]
src = "src"
out = "out"
libs = ["lib"]
solc = "0.8.10" # to use a specific local solc install set the path as `solc = "<path to solc>/solc"`
eth-rpc-url = "https://mainnet.infura.io"

## set only when the `hardhat` profile is selected
[profile.hardhat]
src = "contracts"
out = "artifacts"
libs = ["node_modules"]

## set only when the `spells` profile is selected
[profile.spells]
## --snip-- more settings
```

## Default profile

When determining the profile to use, `Config` considers the following sources in ascending priority order to read from
and merge, at the per-key level:

1. [`Config::default()`], which provides default values for all parameters.
2. `foundry.toml` _or_ TOML file path in `FOUNDRY_CONFIG` environment variable.
3. `FOUNDRY_` or `DAPP_` prefixed environment variables.

The selected profile is the value of the `FOUNDRY_PROFILE` environment variable, or if it is not set, "default".

## Configuration Reference

For a full list of all configuration options, see the [Foundry Book](https://getfoundry.sh/config/reference/overview):

- [Default Configuration](https://getfoundry.sh/config/reference/default-config) - All `[profile.default]` options
- [Testing Configuration](https://getfoundry.sh/config/reference/testing) - Fuzz and invariant testing options
- [Formatter Configuration](https://getfoundry.sh/config/reference/formatter) - Code formatting options
- [Linter Configuration](https://getfoundry.sh/config/reference/linter) - Linting options
- [Doc Generator Configuration](https://getfoundry.sh/config/reference/doc-generator) - Documentation generation options

## Environment Variables

Foundry's tools read all environment variable names prefixed with `FOUNDRY_` using the string after the `_` as the name
of a configuration value as the value of the parameter as the value itself. But the
corresponding [dapptools](https://github.com/dapphub/dapptools/tree/master/src/dapp#configuration) config vars are also
supported, this means that `FOUNDRY_SRC` and `DAPP_SRC` are equivalent.

Some exceptions to the above are [explicitly ignored](https://github.com/foundry-rs/foundry/blob/10440422e63aae660104e079dfccd5b0ae5fd720/config/src/lib.rs#L1539-L15522) due to security concerns.

Environment variables take precedence over values in `foundry.toml`. Values are parsed as a loose form of TOML syntax.
