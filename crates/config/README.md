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
auto_detect_remappings = true
remappings = []
# list of libraries to link: `"src/MyLib.sol:MyLib:0x..."`
libraries = []
cache = true
cache_path = 'cache'
broadcast = 'broadcast'
snapshots = 'snapshots'
# whether to check for differences against stored gas snapshots
gas_snapshot_check = false
gas_snapshot_emit = true
allow_paths = []
include_paths = []
# glob patterns for paths to skip
skip = []
force = false
# whether to dynamically link tests
dynamic_test_linking = false
evm_version = 'osaka'
gas_reports = ['*']
gas_reports_ignore = []
gas_reports_include_tests = false
## Sets the concrete solc version to use, this overrides the `auto_detect_solc` value
# solc = '0.8.10'
auto_detect_solc = true
offline = false
optimizer = false
optimizer_runs = 200
model_checker = { contracts = { 'a.sol' = ['A1', 'A2'] }, engine = 'chc', targets = ['assert'], timeout = 10000 }
verbosity = 0
eth_rpc_url = "https://example.com/"
eth_rpc_accept_invalid_certs = false
eth_rpc_no_proxy = false
# eth_rpc_jwt = "secret"
# eth_rpc_timeout = 30
# eth_rpc_headers = ["x-custom:value"]
etherscan_api_key = "YOURETHERSCANAPIKEY"
# known error codes: ["unreachable", "unused-return", "unused-param", "unused-var", "code-size", "shadowing", "func-mutability", "license", "pragma-solidity", "virtual-interfaces", "same-varname", "too-many-warnings", "constructor-visibility", "init-code-size", "missing-receive-ether", "unnamed-return", "transient-storage"]
ignored_error_codes = ["license", "code-size"]
ignored_warnings_from = ["path_to_ignore"]
# "never", "warnings", or "notes"
deny = "never"
match_test = "Foo"
no_match_test = "Bar"
match_contract = "Foo"
no_match_contract = "Bar"
match_path = "*/Foo*"
no_match_path = "*/Bar*"
no_match_coverage = "Baz"
test_failures_file = "cache/test-failures"
# 0 defaults to the number of logical cores
threads = 0
show_progress = true
ffi = false
allow_internal_expect_revert = false
always_use_create_2_factory = false
prompt_timeout = 120
sender = '0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38'
tx_origin = '0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38'
initial_balance = '0xffffffffffffffffffffffff'
block_number = 0
fork_block_number = 0
chain_id = 1
# NOTE: use a string if gas_limit exceeds 2**63-1
gas_limit = 1073741824
code_size_limit = 24576
gas_price = 0
block_base_fee_per_gas = 0
block_coinbase = '0x0000000000000000000000000000000000000000'
block_timestamp = 0
block_difficulty = 0
block_prevrandao = '0x0000000000000000000000000000000000000000000000000000000000000000'
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
rpc_storage_caching = { chains = "all", endpoints = "all" }
no_storage_caching = false
no_rpc_rate_limit = false
use_literal_content = false
# use "none" for deterministic code
bytecode_hash = "ipfs"
cbor_metadata = true
# "default", "strip", "debug", "verboseDebug"
revert_strings = "default"
sparse_mode = false
build_info = false
build_info_path = "build-info"
# Configures permissions for cheatcodes that touch the filesystem
# access: "read-write", "read", "write", "none"
fs_permissions = [{ access = "read", path = "./out"}]
isolate = false
disable_block_gas_limit = false
enable_tx_gas_limit = false
unchecked_cheatcode_artifacts = false
create2_library_salt = '0x0000000000000000000000000000000000000000000000000000000000000000'
create2_deployer = '0x4e59b44847b379578588920ca78fbf26c0b4956c'
assertions_revert = true
legacy_assertions = false
transaction_timeout = 120

[fuzz]
runs = 256
fail_on_revert = true
max_test_rejects = 65536
seed = '0x3e8'
gas_report_samples = 256
show_logs = false
# timeout = 60
# failure_persist_dir = 'cache/fuzz'
dictionary_weight = 40
include_storage = true
include_push_bytes = true
max_fuzz_dictionary_addresses = 15728640
max_fuzz_dictionary_values = 9830400
max_fuzz_dictionary_literals = 6553600
# corpus_dir = 'corpus'
corpus_gzip = true
corpus_min_mutations = 5
corpus_min_size = 0
show_edge_coverage = false

[invariant]
runs = 256
depth = 500
fail_on_revert = false
call_override = false
shrink_run_limit = 5000
max_assume_rejects = 65536
gas_report_samples = 256
show_metrics = true
# timeout = 60
show_solidity = false
# max_time_delay = 86400
# max_block_delay = 10000
check_interval = 1
# failure_persist_dir = 'cache/invariant'
dictionary_weight = 80
include_storage = true
include_push_bytes = true
max_fuzz_dictionary_addresses = 15728640
max_fuzz_dictionary_values = 9830400
max_fuzz_dictionary_literals = 6553600
# corpus_dir = 'corpus'
corpus_gzip = true
corpus_min_mutations = 5
corpus_min_size = 0
show_edge_coverage = false

[fmt]
line_length = 120
tab_width = 4
# "space" or "tab"
style = "space"
bracket_spacing = false
# "preserve", "long", "short"
int_types = "long"
# "params_always", "params_first_multi", "attributes_first", "all", "all_params"
multiline_func_header = "attributes_first"
# "preserve", "double", "single"
quote_style = "double"
# "preserve", "remove", "thousands"
number_underscore = "preserve"
# "preserve", "remove", "bytes"
hex_underscore = "remove"
# "preserve", "single", "multi"
single_line_statement_blocks = "preserve"
override_spacing = false
wrap_comments = false
# "preserve", "line", "block"
docs_style = "preserve"
ignore = []
contract_new_lines = false
sort_imports = false
# "prefer_plain", "prefer_glob", "preserve"
namespace_import_style = "prefer_plain"
pow_no_space = false
# "none", "calls", "events", "errors", "events_errors", "all"
prefer_compact = "all"
single_line_imports = false

[doc]
out = "docs"
title = ""
book = "book.toml"
# homepage = "README.md"
# repository = "https://github.com/..."
# path = "tree/main/src"
ignore = []

[lint]
lint_on_build = true
# "high", "medium", "low", "info", "gas", "code-size"
severity = ["high", "medium", "low"]
exclude_lints = []
ignore = []
mixed_case_exceptions = ["ERC", "URI"]
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
