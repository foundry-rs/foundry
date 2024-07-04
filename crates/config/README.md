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

### All Options

The following is a foundry.toml file with all configuration options set. See also [/config/src/lib.rs](./src/lib.rs) and [/cli/tests/it/config.rs](../forge/tests/it/config.rs).

```toml
## defaults for _all_ profiles
[profile.default]
src = 'src'
test = 'test'
script = 'script'
out = 'out'
libs = ['lib']
auto_detect_remappings = true # recursive auto-detection of remappings
remappings = []
# list of libraries to link in the form of `<path to lib>:<lib name>:<address>`: `"src/MyLib.sol:MyLib:0x8De6DDbCd5053d32292AAA0D2105A32d108484a6"`
# the <path to lib> supports remappings
libraries = []
cache = true
cache_path = 'cache'
broadcast = 'broadcast'
# additional solc allow paths
allow_paths = []
# additional solc include paths
include_paths = []
force = false
evm_version = 'shanghai'
gas_reports = ['*']
gas_reports_ignore = []
## Sets the concrete solc version to use, this overrides the `auto_detect_solc` value
# solc = '0.8.10'
auto_detect_solc = true
offline = false
optimizer = true
optimizer_runs = 200
model_checker = { contracts = { 'a.sol' = [
    'A1',
    'A2',
], 'b.sol' = [
    'B1',
    'B2',
] }, engine = 'chc', targets = [
    'assert',
    'outOfBounds',
], timeout = 10000 }
verbosity = 0
eth_rpc_url = "https://example.com/"
# Setting this option enables decoding of error traces from mainnet deployed / verfied contracts via etherscan
etherscan_api_key = "YOURETHERSCANAPIKEY"
# ignore solc warnings for missing license and exceeded contract size
# known error codes are: ["unreachable", "unused-return", "unused-param", "unused-var", "code-size", "shadowing", "func-mutability", "license", "pragma-solidity", "virtual-interfaces", "same-varname", "too-many-warnings", "constructor-visibility", "init-code-size", "missing-receive-ether", "unnamed-return", "transient-storage"]
# additional warnings can be added using their numeric error code: ["license", 1337]
ignored_error_codes = ["license", "code-size"]
ignored_warnings_from = ["path_to_ignore"]
deny_warnings = false
match_test = "Foo"
no_match_test = "Bar"
match_contract = "Foo"
no_match_contract = "Bar"
match_path = "*/Foo*"
no_match_path = "*/Bar*"
no_match_coverage = "Baz"
# Number of threads to use. Not set or zero specifies the number of logical cores.
threads = 0
# whether to show test execution progress
show_progress = true
ffi = false
always_use_create_2_factory = false
prompt_timeout = 120
# These are the default callers, generated using `address(uint160(uint256(keccak256("foundry default caller"))))`
sender = '0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38'
tx_origin = '0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38'
initial_balance = '0xffffffffffffffffffffffff'
block_number = 0
fork_block_number = 0
chain_id = 1
# NOTE due to a toml-rs limitation, this value needs to be a string if the desired gas limit exceeds 2**63-1 (9223372036854775807).
# `gas_limit = "max"` is equivalent to `gas_limit = "18446744073709551615"`. This is not recommended
# as it will make infinite loops effectively hang during execution.
gas_limit = 1073741824
gas_price = 0
block_base_fee_per_gas = 0
block_coinbase = '0x0000000000000000000000000000000000000000'
block_timestamp = 0
block_difficulty = 0
block_prevrandao = '0x0000000000000000000000000000000000000000'
block_gas_limit = 30000000
memory_limit = 134217728
extra_output = ["metadata"]
extra_output_files = []
names = false
sizes = false
via_ir = false
ast = false
# caches storage retrieved locally for certain chains and endpoints
# can also be restricted to `chains = ["optimism", "mainnet"]`
# by default all endpoints will be cached, alternative options are "remote" for only caching non localhost endpoints and "<regex>"
# to disable storage caching entirely set `no_storage_caching = true`
rpc_storage_caching = { chains = "all", endpoints = "all" }
# this overrides `rpc_storage_caching` entirely
no_storage_caching = false
# Whether to store the referenced sources in the metadata as literal data.
use_literal_content = false
# use ipfs method to generate the metadata hash, solc's default.
# To not include the metadata hash, to allow for deterministic code: https://docs.soliditylang.org/en/latest/metadata.html, use "none"
bytecode_hash = "ipfs"
# Whether to append the metadata hash to the bytecode
cbor_metadata = true
# How to treat revert (and require) reason strings.
# Possible values are: "default", "strip", "debug" and "verboseDebug".
#  "default" does not inject compiler-generated revert strings and keeps user-supplied ones.
# "strip" removes all revert strings (if possible, i.e. if literals are used) keeping side-effects
# "debug" injects strings for compiler-generated internal reverts, implemented for ABI encoders V1 and V2 for now.
# "verboseDebug" even appends further information to user-supplied revert strings (not yet implemented)
revert_strings = "default"
# If this option is enabled, Solc is instructed to generate output (bytecode) only for the required contracts
# this can reduce compile time for `forge test` a bit but is considered experimental at this point.
sparse_mode = false
build_info = true
build_info_path = "build-info"
root = "root"
# Configures permissions for cheatcodes that touch the filesystem like `vm.writeFile`
# `access` restricts how the `path` can be accessed via cheatcodes
#    `read-write` | `true`   => `read` + `write` access allowed (`vm.readFile` + `vm.writeFile`)
#    `none`| `false` => no access
#    `read` => only read access (`vm.readFile`)
#    `write` => only write access (`vm.writeFile`)
# The `allowed_paths` further lists the paths that are considered, e.g. `./` represents the project root directory
# By default, only read access is granted to the project's out dir, so generated artifacts can be read by default
# following example enables read-write access for the project dir :
#       `fs_permissions = [{ access = "read-write", path = "./"}]`
fs_permissions = [{ access = "read", path = "./out"}]
# whether failed assertions should revert
# note that this only applies to native (cheatcode) assertions, invoked on Vm contract
assertions_revert = true
# whether `failed()` should be invoked to check if the test have failed
legacy_assertions = false
[fuzz]
runs = 256
max_test_rejects = 65536
seed = '0x3e8'
dictionary_weight = 40
include_storage = true
include_push_bytes = true

[invariant]
runs = 256
depth = 500
fail_on_revert = false
call_override = false
dictionary_weight = 80
include_storage = true
include_push_bytes = true
shrink_run_limit = 5000

[fmt]
line_length = 100
tab_width = 2
bracket_spacing = true
```

#### Additional Optimizer settings

Optimizer components can be tweaked with the `OptimizerDetails` object:

See [Compiler Input Description `settings.optimizer.details`](https://docs.soliditylang.org/en/latest/using-the-compiler.html#compiler-input-and-output-json-description)

The `optimizer_details` (`optimizerDetails` also works) settings must be prefixed with the profile they correspond
to: `[profile.default.optimizer_details]`
belongs to the `[profile.default]` profile

```toml
[profile.default.optimizer_details]
constantOptimizer = true
yul = true
# this sets the `yulDetails` of the `optimizer_details` for the `default` profile
[profile.default.optimizer_details.yulDetails]
stackAllocation = true
optimizerSteps = 'dhfoDgvulfnTUtnIf'
```

#### RPC-Endpoints settings

The `rpc_endpoints` value accepts a list of `alias = "<url|env var>"` pairs.

The following example declares two pairs:
The alias `optimism` references the endpoint URL directly.
The alias `mainnet` references the environment variable `RPC_MAINNET` which holds the entire URL.
The alias `goerli` references an endpoint that will be interpolated with the value the `GOERLI_API_KEY` holds.

Environment variables need to be wrapped in `${}`

```toml
[rpc_endpoints]
optimism = "https://optimism.alchemyapi.io/v2/1234567"
mainnet = "${RPC_MAINNET}"
goerli = "https://eth-goerli.alchemyapi.io/v2/${GOERLI_API_KEY}"
```

#### Etherscan API Key settings

The `etherscan` value accepts a list of `alias = "{key = "", url? ="", chain?= """""}"` items.

the `key` attribute is always required and should contain the actual API key for that chain or an env var that holds the key in the form `${ENV_VAR}`
The `chain` attribute is optional if the `alias` is the already the `chain` name, such as in `mainnet = { key = "${ETHERSCAN_MAINNET_KEY}"}`
The optional `url` attribute can be used to explicitly set the Etherscan API url, this is the recommended setting for chains not natively supported by name.

```toml
[etherscan]
mainnet = { key = "${ETHERSCAN_MAINNET_KEY}" }
mainnet2 = { key = "ABCDEFG", chain = "mainnet" }
optimism = { key = "1234576", chain = 42 }
unknownchain = { key = "ABCDEFG", url = "https://<etherscan-api-url-for-that-chain>" }
```

##### Additional Model Checker settings

[Solidity's built-in model checker](https://docs.soliditylang.org/en/latest/smtchecker.html#tutorial)
is an opt-in module that can be enabled via the `ModelChecker` object.

See [Compiler Input Description `settings.modelChecker`](https://docs.soliditylang.org/en/latest/using-the-compiler.html#compiler-input-and-output-json-description)
and [the model checker's options](https://docs.soliditylang.org/en/latest/smtchecker.html#smtchecker-options-and-tuning).

The module is available in `solc` release binaries for OSX and Linux.
The latter requires the z3 library version [4.8.8, 4.8.14] to be installed
in the system (SO version 4.8).

Similarly to the optimizer settings above, the `model_checker` settings must be
prefixed with the profile they correspond to: `[profile.default.model_checker]` belongs
to the `[profile.default]` profile.

```toml
[profile.default.model_checker]
contracts = { 'src/Contract.sol' = [ 'Contract' ] }
engine = 'chc'
timeout = 10000
targets = [ 'assert' ]
```

The fields above are recommended when using the model checker.
Setting which contract should be verified is extremely important, otherwise all
available contracts will be verified which can consume a lot of time.
The recommended engine is `chc`, but `bmc` and `all` (runs both) are also
accepted.
It is also important to set a proper timeout (given in milliseconds), since the
default time given to the underlying solvers may not be enough.
If no verification targets are given, only assertions will be checked.

The model checker will run when `forge build` is invoked, and will show
findings as warnings if any.

## Environment Variables

Foundry's tools read all environment variable names prefixed with `FOUNDRY_` using the string after the `_` as the name
of a configuration value as the value of the parameter as the value itself. But the
corresponding [dapptools](https://github.com/dapphub/dapptools/tree/master/src/dapp#configuration) config vars are also
supported, this means that `FOUNDRY_SRC` and `DAPP_SRC` are equivalent.

Some exceptions to the above are [explicitly ignored](https://github.com/foundry-rs/foundry/blob/10440422e63aae660104e079dfccd5b0ae5fd720/config/src/lib.rs#L1539-L15522) due to security concerns.

Environment variables take precedence over values in `foundry.toml`. Values are parsed as a loose form of TOML syntax.
Consider the following examples:
