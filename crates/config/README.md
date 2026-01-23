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
# =============================================================================
# FOUNDRY COMPLETE CONFIGURATION REFERENCE
# =============================================================================
# This file documents ALL available configuration options for foundry.toml
# Generated from the Rust Config structs in foundry/crates/config/src/
# =============================================================================

[profile.default]

# =============================================================================
# PROJECT PATHS
# =============================================================================

# Path of the sources directory
# Default: "src"
src = "src"

# Path of the tests directory
# Default: "test"
test = "test"

# Path of the scripts directory
# Default: "script"
script = "script"

# Path to the artifacts directory
# Default: "out"
out = "out"

# Paths to all library folders, such as "lib" or "node_modules"
# Default: ["lib"]
libs = ["lib"]

# Path to the cache store
# Default: "cache"
cache_path = "cache"

# Path to store broadcast logs
# Default: "broadcast"
broadcast = "broadcast"

# Where the gas snapshots are stored
# Default: "snapshots"
snapshots = "snapshots"

# Path where last test run failures are recorded
# Default: "cache/test-failures"
test_failures_file = "cache/test-failures"

# =============================================================================
# REMAPPINGS & LIBRARIES
# =============================================================================

# Remappings to use for this repo
# Format: ["@openzeppelin/=lib/openzeppelin-contracts/"]
remappings = []

# Whether to autodetect remappings by scanning the libs folders
# Default: true
auto_detect_remappings = true

# Library addresses to link
# Format: ["src/MyLib.sol:MyLib:0x..."]
libraries = []

# Additional paths passed to solc --allow-paths
allow_paths = []

# Additional paths passed to solc --include-path
include_paths = []

# Glob patterns for file paths to skip when building and executing contracts
# Example: ["test/invariant/**/*", "script/**/*"]
skip = []

# =============================================================================
# BUILD & CACHE
# =============================================================================

# Whether to enable the build cache
# Default: true
cache = true

# Whether to dynamically link tests
# Default: false
dynamic_test_linking = false

# Whether to forcefully clean all project artifacts before running commands
# Default: false
force = false

# Whether to compile in sparse mode
# If enabled, only required contracts/files will be selected for solc's output
# Default: false
sparse_mode = false

# Generates additional build info json files for every new build
# Contains the CompilerInput and CompilerOutput
# Default: false
build_info = false

# The path to the build-info directory that contains the build info json files
# build_info_path = "build-info"

# =============================================================================
# GAS SNAPSHOTS
# =============================================================================

# Whether to check for differences against previously stored gas snapshots
# Default: false
gas_snapshot_check = false

# Whether to emit gas snapshots to disk
# Default: true
gas_snapshot_emit = true

# =============================================================================
# SOLIDITY COMPILER
# =============================================================================

# The Solc instance to use. Takes precedence over auto_detect_solc
# Can be a version string like "0.8.20" or path to solc binary
# solc = "0.8.20"

# Whether to autodetect the solc compiler version to use
# Default: true
auto_detect_solc = true

# Offline mode - if set, network access (downloading solc) is disallowed
# If auto_detect_solc = true and offline = true, required solc versions will
# be auto detected but will not be installed if missing
# Default: false
offline = false

# The EVM version to use when building contracts
# Options: "homestead", "tangerineWhistle", "spuriousDragon", "byzantium",
#          "constantinople", "petersburg", "istanbul", "berlin", "london",
#          "paris", "shanghai", "cancun", "osaka"
# Default: "osaka"
evm_version = "osaka"

# Whether to activate optimizer
# optimizer = true

# The number of runs specifies roughly how often each opcode will be executed
# Trade-off between code size (deploy cost) and execution cost
# optimizer_runs = 1 produces short but expensive code
# Higher values produce longer but more gas efficient code
# Maximum value: 2^32-1
# optimizer_runs = 200

# Switch optimizer components on or off in detail
# [profile.default.optimizer_details]
# peephole = true
# inliner = true
# jumpdestRemover = true
# orderLiterals = true
# deduplicate = true
# cse = true
# constantOptimizer = true
# yul = true

# Model checker settings for formal verification
# [profile.default.model_checker]
# contracts = {}
# engine = "chc"
# timeout = 10000

# If set to true, changes compilation pipeline to go through Yul IR
# Default: false
via_ir = false

# Whether to include the AST as JSON in the compiler output
# Default: false
ast = false

# Whether to store the referenced sources in the metadata as literal data
# Default: false
use_literal_content = false

# Whether to include the metadata hash
# Options: "none", "ipfs", "bzzr1"
# Set to "none" for deterministic code (machine-independent)
# Default: "ipfs"
bytecode_hash = "ipfs"

# Whether to append the metadata hash to the bytecode
# If false and bytecode_hash is not "none", solc will issue a warning
# Default: true
cbor_metadata = true

# How to treat revert (and require) reason strings
# Options: "default", "strip", "debug", "verboseDebug"
# revert_strings = "default"

# Additional output selection for all contracts
# Examples: "ir", "irOptimized", "devdoc", "userdoc", "storageLayout", "ewasm"
# See Solc Compiler API for full list
extra_output = []

# Additional output files to emit for every contract
# Difference from extra_output: emits as separate files instead of in artifact
# Example: ["metadata"] creates metadata.json for each contract
extra_output_files = []

# Whether to print the names of the compiled contracts
# Default: false
names = false

# Whether to print the sizes of the compiled contracts
# Default: false
sizes = false

# Optional additional CLI arguments to pass to solc binary
extra_args = []

# =============================================================================
# ERROR HANDLING
# =============================================================================

# List of solidity error codes to always silence in compiler output
# Default: [1878, 5574, 5660, 2394, 5733, 3199] (common warnings)
ignored_error_codes = [1878, 5574, 5660, 2394, 5733, 3199]

# List of file paths to ignore warnings from
ignored_warnings_from = []

# Diagnostic level (minimum) at which the process should finish with non-zero exit
# Options: "never", "warnings", "notes"
# Default: "never"
deny = "never"

# DEPRECATED: use `deny` instead
# deny_warnings = false

# =============================================================================
# TESTING
# =============================================================================

# The address which will be executing all tests
# Default: 0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38
sender = "0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38"

# The tx.origin value during EVM execution
# Default: 0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38
tx_origin = "0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38"

# The initial balance of each deployed test contract
# Default: 0xffffffffffffffffffffffff (2^96 - 1)
initial_balance = "0xffffffffffffffffffffffff"

# Only run test functions matching the specified regex pattern
# match_test = "test_.*"

# Only run test functions that do not match the specified regex pattern
# no_match_test = "testFail.*"

# Only run tests in contracts matching the specified regex pattern
# match_contract = ".*Test"

# Only run tests in contracts that do not match the specified regex pattern
# no_match_contract = ".*Invariant.*"

# Only run tests in source files matching the specified glob pattern
# match_path = "test/unit/*"

# Only run tests in source files that do not match the specified glob pattern
# no_match_path = "test/integration/*"

# Only show coverage for files that do not match the specified regex pattern
# no_match_coverage = "test/.*"

# Max concurrent threads to use
# threads = 4

# Whether to show test execution progress
# Default: false
show_progress = false

# Whether to allow ffi cheatcodes in tests
# Default: false
ffi = false

# Whether to allow expectRevert for internal functions
# Default: false
allow_internal_expect_revert = false

# Use the CREATE2 factory in all cases including tests and non-broadcasting scripts
# Default: false
always_use_create_2_factory = false

# Sets a timeout in seconds for vm.prompt cheatcodes
# Default: 120
prompt_timeout = 120

# Whether failed assertions should revert (only for native cheatcode assertions)
# Default: true
assertions_revert = true

# Whether failed() should be invoked to check if the test has failed
# Default: false
legacy_assertions = false

# =============================================================================
# EVM CONFIGURATION
# =============================================================================

# The block.number value during EVM execution
# Default: 1
block_number = 1

# Pins the block number for the state fork
# fork_block_number = 12345678

# The chain name or EIP-155 chain ID
# chain = 1

# Block gas limit
# Default: ~1B (1 << 30)
gas_limit = 1073741824

# EIP-170: Contract code size limit in bytes
# Useful to increase for tests
# code_size_limit = 24576

# tx.gasprice value during EVM execution
# If not set, uses remote client's gas price in fork mode
# gas_price = 0

# The base fee in a block
# Default: 0
block_base_fee_per_gas = 0

# The block.coinbase value during EVM execution
# Default: 0x0000000000000000000000000000000000000000
block_coinbase = "0x0000000000000000000000000000000000000000"

# The block.timestamp value during EVM execution
# Default: 1
block_timestamp = 1

# The block.difficulty value during EVM execution
# Default: 0
block_difficulty = 0

# Before merge: block.max_hash, after merge: block.prevrandao
# Default: 0x0000000000000000000000000000000000000000000000000000000000000000
block_prevrandao = "0x0000000000000000000000000000000000000000000000000000000000000000"

# The block.gaslimit value during EVM execution
# block_gas_limit = 30000000

# The memory limit per EVM execution in bytes
# If exceeded, a MemoryLimitOOG result is thrown
# Default: 128MiB (134217728 bytes, 1 << 27)
memory_limit = 134217728

# Whether to enable call isolation
# Useful for more correct gas accounting and EVM behavior
# Default: false (unless isolate-by-default feature is enabled)
isolate = false

# Whether to disable the block gas limit checks
# Default: false
disable_block_gas_limit = false

# Whether to enable the tx gas limit checks as imposed by Osaka (EIP-7825)
# Default: false
enable_tx_gas_limit = false

# =============================================================================
# RPC CONFIGURATION
# =============================================================================

# URL of the RPC server that should be used for any RPC calls
# eth_rpc_url = "https://eth-mainnet.alchemyapi.io/v2/YOUR_KEY"

# Whether to accept invalid certificates for the RPC server
# Default: false
eth_rpc_accept_invalid_certs = false

# Whether to disable automatic proxy detection for the RPC server
# Helps in sandboxed environments where system proxy detection causes crashes
# Default: false
eth_rpc_no_proxy = false

# JWT secret for RPC authentication
# eth_rpc_jwt = "your-jwt-secret"

# Timeout in seconds for RPC calls
# eth_rpc_timeout = 30

# Headers to include in RPC calls
# Format: ["x-custom-header:value", "x-another-header:another-value"]
# eth_rpc_headers = []

# Etherscan API key, or alias for an EtherscanConfig in etherscan table
# etherscan_api_key = "YOUR_API_KEY"

# RPC storage caching settings
[profile.default.rpc_storage_caching]
# Which chains to cache (e.g., "all", "mainnet,optimism", or chain IDs)
chains = "all"
# Which endpoints to cache ("all", "remote", or specific URLs)
endpoints = "remote"

# Disables storage caching entirely (overrides rpc_storage_caching)
# Default: false
# no_storage_caching = false

# Disables rate limiting entirely (overrides compute_units_per_second)
# Default: false
# no_rpc_rate_limit = false

# Multiple RPC endpoints and their aliases
# [profile.default.rpc_endpoints]
# mainnet = "https://eth-mainnet.alchemyapi.io/v2/${ALCHEMY_KEY}"
# optimism = "https://opt-mainnet.g.alchemy.com/v2/${ALCHEMY_KEY}"

# Multiple Etherscan API configs and their aliases
# [profile.default.etherscan]
# mainnet = { key = "${ETHERSCAN_API_KEY}" }
# optimism = { key = "${OPTIMISM_ETHERSCAN_KEY}", chain = "optimism" }

# =============================================================================
# GAS REPORTS
# =============================================================================

# List of contracts to generate gas reports for
# Use ["*"] for all contracts
# Default: ["*"]
gas_reports = ["*"]

# List of contracts to ignore for gas reports
gas_reports_ignore = []

# Whether to include gas reports for tests
# Default: false
gas_reports_include_tests = false

# =============================================================================
# CREATE2 CONFIGURATION
# =============================================================================

# CREATE2 salt to use for library deployment in scripts
# Default: 0x0000000000000000000000000000000000000000000000000000000000000000
create2_library_salt = "0x0000000000000000000000000000000000000000000000000000000000000000"

# The CREATE2 deployer address to use
# Default: 0x4e59b44847b379578588920ca78fbf26c0b4956c
create2_deployer = "0x4e59b44847b379578588920ca78fbf26c0b4956c"

# =============================================================================
# FILE SYSTEM PERMISSIONS
# =============================================================================

# Configures permissions of cheat codes that touch the file system
# Specifies what operations can be executed (read, write)
[profile.default.fs_permissions]
# Default: read access to "out" directory
# read = ["out"]
# read-write = ["cache", "broadcast"]

# =============================================================================
# ADDRESS LABELS
# =============================================================================

# Address labels for better trace output
# [profile.default.labels]
# "0x1234..." = "MyContract"
# "0xabcd..." = "Treasury"

# =============================================================================
# CHEATCODE CONFIGURATION
# =============================================================================

# Verbosity level (0-5)
# Default: 0
verbosity = 0

# Whether to enable safety checks for vm.getCode and vm.getDeployedCode
# If disabled, it's possible to access artifacts which were not recompiled/cached
# Default: false
unchecked_cheatcode_artifacts = false

# =============================================================================
# TRANSACTION CONFIGURATION
# =============================================================================

# Timeout for transactions in seconds
# Default: 120
transaction_timeout = 120

# Whether to enable script execution protection
# Default: true
script_execution_protection = true

# =============================================================================
# VYPER CONFIGURATION
# =============================================================================

# [profile.default.vyper]
# path = "vyper"  # Path to vyper binary

# =============================================================================
# FUZZ TESTING CONFIGURATION
# =============================================================================

[fuzz]
# The number of test cases that must execute for each property test
# Default: 256
runs = 256

# Fails the fuzzed test if a revert occurs
# Default: true
fail_on_revert = true

# The maximum number of test case rejections allowed
# Encountered during usage of vm.assume cheatcode
# Default: 65536
max_test_rejects = 65536

# Optional seed for the fuzzing RNG algorithm
# seed = "0x..."

# Number of runs to execute and include in the gas report
# Default: 256
gas_report_samples = 256

# Path where fuzz failures are recorded and replayed
# Default: "cache/fuzz"
failure_persist_dir = "cache/fuzz"

# Show console.log in fuzz test
# Default: false
show_logs = false

# Optional timeout (in seconds) for each property test
# timeout = 60

# --- Fuzz Dictionary Configuration ---

# The weight of the dictionary (percentage 0-100)
# Default: 40
dictionary_weight = 40

# Whether to include values from storage
# Default: true
include_storage = true

# Whether to include push bytes values
# Default: true
include_push_bytes = true

# Maximum addresses to record in dictionary
# Once exceeded, starts evicting random entries to prevent memory blowup
# Default: ~15.7M (300MB / 20 bytes)
max_fuzz_dictionary_addresses = 15728640

# Maximum values to record in dictionary
# Once exceeded, starts evicting random entries
# Default: ~9.8M (300MB / 32 bytes)
max_fuzz_dictionary_values = 9830400

# Maximum literal values to seed from the AST
# Independent from max addresses and values
# Default: ~6.5M (200MB / 32 bytes)
max_fuzz_dictionary_literals = 6553600

# --- Fuzz Corpus Configuration ---

# Path to corpus directory, enables coverage-guided fuzzing mode
# If not set, sequences producing new coverage are not persisted and mutated
# corpus_dir = "corpus/fuzz"

# Whether corpus uses gzip file compression and decompression
# Default: true
corpus_gzip = true

# Number of mutations until entry marked as eligible to be flushed from memory
# Mutations will be performed at least this many times
# Default: 5
corpus_min_mutations = 5

# Number of corpus entries that won't be evicted from memory
# Default: 0
corpus_min_size = 0

# Whether to collect and display edge coverage metrics
# Default: false
show_edge_coverage = false

# =============================================================================
# INVARIANT TESTING CONFIGURATION
# =============================================================================

[invariant]
# The number of runs that must execute for each invariant test group
# Default: 256
runs = 256

# The number of calls executed to attempt to break invariants in one run
# Default: 500
depth = 500

# Fails the invariant fuzzing if a revert occurs
# Default: false
fail_on_revert = false

# Allows overriding an unsafe external call when running invariant tests
# e.g., reentrancy checks
# Default: false
call_override = false

# The maximum number of attempts to shrink the sequence
# Default: 5000
shrink_run_limit = 5000

# The maximum number of rejects via vm.assume in a single invariant run
# Default: 65536
max_assume_rejects = 65536

# Number of runs to execute and include in the gas report
# Default: 256
gas_report_samples = 256

# Path where invariant failures are recorded and replayed
# Default: "cache/invariant"
failure_persist_dir = "cache/invariant"

# Whether to collect and display fuzzed selectors metrics
# Default: true
show_metrics = true

# Optional timeout (in seconds) for each invariant test
# timeout = 300

# Display counterexample as solidity calls
# Default: false
show_solidity = false

# Maximum time (in seconds) between generated transactions
# max_time_delay = 86400

# Maximum number of blocks elapsed between generated transactions
# max_block_delay = 1000

# Number of calls to execute between invariant assertions
# 0: Only assert on the last call of each run (fastest, may miss exact breaking call)
# 1 (default): Assert after every call (most precise)
# N: Assert every N calls AND always on the last call
# Default: 1
check_interval = 1

# --- Invariant Dictionary Configuration ---

# The weight of the dictionary (percentage 0-100)
# Default: 80 (higher than fuzz default)
dictionary_weight = 80

# Whether to include values from storage
# Default: true
include_storage = true

# Whether to include push bytes values
# Default: true
include_push_bytes = true

# Maximum addresses to record in dictionary
max_fuzz_dictionary_addresses = 15728640

# Maximum values to record in dictionary
max_fuzz_dictionary_values = 9830400

# Maximum literal values to seed from the AST
max_fuzz_dictionary_literals = 6553600

# --- Invariant Corpus Configuration ---

# Path to corpus directory, enables coverage-guided fuzzing mode
# corpus_dir = "corpus/invariant"

# Whether corpus uses gzip compression
corpus_gzip = true

# Minimum mutations before entry can be flushed
corpus_min_mutations = 5

# Minimum corpus entries to keep in memory
corpus_min_size = 0

# Whether to collect and display edge coverage metrics
show_edge_coverage = false

# =============================================================================
# FORMATTER CONFIGURATION
# =============================================================================

[fmt]
# Maximum line length where formatter will try to wrap the line
# Default: 120
line_length = 120

# Number of spaces per indentation level (ignored if style is Tab)
# Default: 4
tab_width = 4

# Style of indent
# Options: "space", "tab"
# Default: "space"
style = "space"

# Print spaces between brackets
# Default: false
bracket_spacing = false

# Style of uint/int256 types
# Options: "preserve", "long", "short"
# "preserve": Use the type defined in source code
# "long": Print full length uint256 or int256
# "short": Print alias uint or int
# Default: "long"
int_types = "long"

# Style of multiline function header when it doesn't fit
# Options: "params_always", "params_first_multi", "attributes_first", "all", "all_params"
# Default: "attributes_first"
multiline_func_header = "attributes_first"

# Style of quotation marks
# Options: "preserve", "double", "single"
# Default: "double"
quote_style = "double"

# Style of underscores in number literals
# Options: "preserve", "remove", "thousands"
# "thousands": Add underscore every thousand if > 9999 (e.g., 10000 -> 10_000)
# Default: "preserve"
number_underscore = "preserve"

# Style of underscores in hex literals
# Options: "preserve", "remove", "bytes"
# "bytes": Add underscore as separator between byte boundaries
# Default: "remove"
hex_underscore = "remove"

# Style of single line blocks in statements
# Options: "preserve", "single", "multi"
# Default: "preserve"
single_line_statement_blocks = "preserve"

# Print space in state variable, function, and modifier override attribute
# Default: false
override_spacing = false

# Wrap comments on line_length reached
# Default: false
wrap_comments = false

# Style of doc comments
# Options: "preserve", "line", "block"
# "line": Use single-line style (///)
# "block": Use block style (/** .. */)
# Default: "preserve"
docs_style = "preserve"

# Globs to ignore
ignore = []

# Add new line at start and end of contract declarations
# Default: false
contract_new_lines = false

# Sort import statements alphabetically in groups (groups separated by newline)
# Default: false
sort_imports = false

# Choose between import styles
# Options: "prefer_plain", "prefer_glob", "preserve"
# "prefer_plain": import "a" as name
# "prefer_glob": import * as name from "a"
# Default: "prefer_plain"
namespace_import_style = "prefer_plain"

# Whether to suppress spaces around the power operator (**)
# Default: false
pow_no_space = false

# Style for broken lists - keep elements together before breaking individually
# Options: "none", "calls", "events", "errors", "events_errors", "all"
# Default: "all"
prefer_compact = "all"

# Keep single imports on a single line even if they exceed line length
# Default: false
single_line_imports = false

# =============================================================================
# DOCUMENTATION CONFIGURATION
# =============================================================================

[doc]
# Doc output path
# Default: "docs"
out = "docs"

# The documentation title
title = ""

# Path to user provided book.toml
# Default: "book.toml"
book = "book.toml"

# Path to user provided welcome markdown
# If none provided, defaults to README.md
homepage = "README.md"

# The repository URL
# repository = "https://github.com/user/repo"

# The path to source code (e.g., "tree/main/packages/contracts")
# Useful for monorepos or projects with source code in specific directories
# path = "tree/main/src"

# Globs to ignore
ignore = []

# =============================================================================
# LINTER CONFIGURATION
# =============================================================================

[lint]
# Specifies which lints to run based on severity
# If uninformed, all severities are checked
# Options: "high", "medium", "low", "info", "gas", "code-size"
# Default: ["high", "medium", "low"]
severity = ["high", "medium", "low"]

# Deny specific lints based on their ID (e.g., "mixed-case-function")
exclude_lints = []

# Globs to ignore
ignore = []

# Whether to run linting during forge build
# Default: true
lint_on_build = true

# Patterns excluded from mixedCase lint checks
# Default: ["ERC", "URI"] to allow names like rescueERC20, ERC721TokenReceiver, tokenURI
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
