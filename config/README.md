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
the file is found or the root is reached. But the typical location for the global `foundry.toml` would
be `~/.foundry/foundry.toml`, which is also checked. If the path set in `FOUNDRY_CONFIG` is absolute, no such search
takes place and the absolute path is used directly.

In `foundry.toml` you can define multiple profiles, therefore the file is assumed to be _nested_, so each top-level key
declares a profile and its values configure the profile.

The following is an example of what such a file might look like:

```toml
## defaults for _all_ profiles
[default]
src = "src"
out = "out"
libs = ["lib"]
solc = "0.8.10" # to use a specific local solc install set the path as `solc = "<path to solc>/solc"`
eth-rpc-url = "https://mainnet.infura.io"

## set only when the `hardhat` profile is selected
[hardhat]
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
cache_path = 'cache'
force = false
evm_version = 'london'
gas_reports = ['*']
## Sets the concrete solc version to use, this overrides the `auto_detect_solc` value
# solc_version = '0.8.10'
auto_detect_solc = true
offline = false
optimizer = true
optimizer_runs = 200
verbosity = 0
ignored_error_codes = []
fuzz_runs = 256
ffi = false
sender = '0x00a329c0648769a73afac7f9381e08fb43dbea72'
tx_origin = '0x00a329c0648769a73afac7f9381e08fb43dbea72'
initial_balance = '0xffffffffffffffffffffffff'
block_number = 0
gas_limit = 9223372036854775807
gas_price = 0
block_base_fee_per_gas = 0
block_coinbase = '0x0000000000000000000000000000000000000000'
block_timestamp = 0
block_difficulty = 0
```

##### Additional Optimizer settings

Optimizer components can be tweaked with the `OptimizerDetails` object:

See [Compiler Input Description `settings.optimizer.details`](https://docs.soliditylang.org/en/latest/using-the-compiler.html#compiler-input-and-output-json-description)

The `optimizer_details` (`optimizerDetails` also works) settings must be prefixed with the profile they correspond to: `[default.optimizer_details]`
belongs to the `[default]` profile

```toml
[default.optimizer_details]
constantOptimizer = true
yul = true
# this sets the `yulDetails` of the `optimizer_details` for the `default` profile
[default.optimizer_details.yulDetails]
stackAllocation = true
optimizerSteps = 'dhfoDgvulfnTUtnIf'
```

## Environment Variables

Foundry's tools read all environment variable names prefixed with `FOUNDRY_` using the string after the `_` as the name
of a configuration value as the value of the parameter as the value itself. But the
corresponding [dapptools](https://github.com/dapphub/dapptools/tree/master/src/dapp#configuration) config vars are also
supported, this means that `FOUNDRY_SRC` and `DAPP_SRC` are equivalent.

Environment variables take precedence over values in `foundry.toml`. Values are parsed as loose form of TOML syntax.
Consider the following examples:
