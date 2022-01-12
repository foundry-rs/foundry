# Configuration

Foundry's configuration system allows you to configure it's tools the way _you_ want while also providing with a
sensible set of defaults.

## Profiles

Configurations can be arbitrarily namespaced by profiles. Foundry's default config is also named `default`, but can
arbitrarily name and configure profiles as you like and set the `FOUNDRY_PROFILE` environment variable to the selected
profile's name. This results in foundry's tools (forge) preferring the values in the profile with the named that's set
in `FOUNDRY_PROFILE`.

## foundry.toml

Foundry's tools search for a `foundry.toml`  or the filename in a `FOUNDRY_CONFIG` environment variable starting at the
current working directory. If it is not found, the parent directory, its parent directory, and so on are searched until
the file is found or the root is reached. The typical location for the global `foundry.toml` would be `~/foundry.toml`.
If the path set in `FOUNDRY_CONFIG` is absolute, no such search takes place and the absolute path is used directly.

In `foundry.toml` you can define multiple profiles, therefore the file is assumed to be _nested_, so each top-level key
declares a profile and its values configure the profile.

The following is an example of what such a file might look like:

```toml
## defaults for _all_ profiles
[default]
src = "src"
out = "out"
libs = ["lib"]
solc-version = "8.0.10"
eth-rpc-url = "https://mainnet.infura.io"

## set only when the `hardhat` profile is selected
[hardhat]
## override the default settings, but inherit everything else
src = "contracts"
out = "artifacts"
libs = ["node_modules"]

## set only when the `spells` profile is selected
[spells]
## --snip-- more settings
```

## Default profile

When determining the profile to use, `Config` considers the following sources in ascending priority order to read from
and merge, at the per-key level:

1. [`Config::default()`], which provides default values for all parameters.
2. `foundry.toml` _or_ TOML file path in `FOUNDRY_CONFIG` environment variable.
3. `FOUNDRY_` or `DAPP_` prefixed environment variables.

The selected profile is the value of the `FOUNDRY_PROFILE` environment variable, or if it is not set, "default".

#### All Options

The following is a foundry.toml file with all configuration options set.

```toml
## defaults for _all_ profiles
[default]
src = 'src'
test = 'test'
out = 'out'
libs = ['lib']
remappings = []
libraries = []
cache = true
evm_version = 'london'
## Sets the concrete solc version to use, this overrides the `auto_detect_solc` value
# solc_version = '0.8.10'
auto_detect_solc = true
optimizer = false
optimizer_runs = 200
verbosity = 0
ignored_error_codes = []
solc_settings = '''
{
  "*": {
    "": [
      "ast"
    ],
    "*": [
      "abi",
      "evm.bytecode",
      "evm.deployedBytecode",
      "evm.methodIdentifiers"
    ]
  }
}
'''
```

## Environment Variables

Foundry's tools read all environment variable names prefixed with `FOUNDRY_` using the string after the `_` as the name
of a configuration value as the value of the parameter as the value itself. But the
corresponding [dapptools](https://github.com/dapphub/dapptools/tree/master/src/dapp#configuration) config vars are also
supported, this means that `FOUNDRY_SRC` and `DAPP_SRC` are equivalent.

Environment variables take precedence over values in `foundry.toml`. Values are parsed as loose form of TOML syntax.
Consider the following examples: