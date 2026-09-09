use alloy_primitives::{Address, U256, address};
use std::time::Duration;

// HD wallet key derivation
pub(crate) const DEFAULT_DERIVATION_PATH_PREFIX: &str = "m/44'/60'/0'/0/";
pub(crate) const MAX_REMEMBER_KEYS: u32 = 64;
pub(crate) const SYMBOLIC_VM_COMPAT_ADDRESS: Address =
    address!("0xF3993A62377BCd56AE39D773740A5390411E8BC9");

// EVM execution limits
pub(crate) const EVM_STACK_LIMIT: usize = 1024;
pub(crate) const CALL_VALUE_STIPEND: u64 = 2300;

// Symbolic exponentiation limits
pub(crate) const SYMBOLIC_EXP_CONCRETE_EXPONENT_LIMIT: u64 = 32;
pub(crate) const CONCRETE_BASE_SYMBOLIC_EXPONENT_LIMIT: u64 = 256;

// Revert selectors and assertion constants
pub(crate) const PANIC_SELECTOR: [u8; 4] = [0x4e, 0x48, 0x7b, 0x71];
pub(crate) const ERROR_SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];
pub(crate) const ASSERT_PANIC_CODE: U256 = U256::from_limbs([1, 0, 0, 0]);
pub(crate) const ASSERTION_FAILED_PREFIX: &str = "assertion failed";

// ABI encoding lengths
pub(crate) const ABI_SELECTOR_PLUS_WORD_LEN: usize = 36; // selector (4) + one ABI word (32)
pub(crate) const ERROR_DATA_MIN_LEN: usize = 68; // selector (4) + offset (32) + length (32)

// Precompile address layout
pub(crate) const PRECOMPILE_ADDRESS_LEADING_ZEROS: usize = 19;

// Solver subprocess supervision.
pub(crate) const SOLVER_CANCEL_CHECK_INTERVAL: Duration = Duration::from_millis(50);

// Portfolio scheduler launch delays.
pub(crate) const SECOND_PORTFOLIO_SOLVER_DELAY: Duration = Duration::from_millis(100);
pub(crate) const RESCUE_PORTFOLIO_SOLVER_DELAY: Duration = Duration::from_millis(500);

// Portfolio scheduler tuning
pub(crate) const PORTFOLIO_SCHEDULER_HISTORY: usize = 8;
pub(crate) const PORTFOLIO_SCHEDULER_MIN_RECENCY_WEIGHT: i64 = 1;
pub(crate) const PORTFOLIO_SCHEDULER_SPEED_BONUS_CAP_MS: u128 = 100;
pub(crate) const PORTFOLIO_SCHEDULER_MAX_SPEED_BONUS: i64 = 100;

// Solver query cache limits
pub(crate) const SYMBOLIC_SOLVER_SAT_CACHE_MAX_ENTRIES: usize = 4096;
pub(crate) const SYMBOLIC_SOLVER_MODEL_CACHE_MAX_ENTRIES: usize = 512;

// Hard arithmetic witness search limits
pub(crate) const HARD_ARITH_FALLBACK_MAX_VARS: usize = 4;
pub(crate) const HARD_ARITH_FALLBACK_MAX_CANDIDATES_PER_VAR: usize = 24;
pub(crate) const HARD_ARITH_FALLBACK_MAX_ASSIGNMENTS: usize = 50_000;

/// Symbolic solver names with built-in command-line mappings.
pub const BUILTIN_SYMBOLIC_SOLVERS: &[&str] =
    &["z3", "yices", "cvc5", "cvc5-int", "bitwuzla", "bitwuzla-abs"];
