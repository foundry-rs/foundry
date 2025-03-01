use crate::selectors::function_selectors_with_pc;
#[allow(deprecated)]
use crate::{
    function_arguments_alloy, state_mutability, storage, Selector, StateMutability, StorageRecord,
};
use alloy_dyn_abi::DynSolType;

/// Represents a public smart contract function
#[derive(Debug)]
pub struct Function {
    /// Function selector (4 bytes)
    pub selector: Selector,

    /// The starting byte offset within the EVM bytecode for the function body
    pub bytecode_offset: usize,

    /// Function arguments
    pub arguments: Option<Vec<DynSolType>>,

    /// State mutability
    pub state_mutability: Option<StateMutability>,
}

/// Contains analyzed information about a smart contract
#[derive(Debug)]
pub struct Contract {
    /// List of contract functions with their metadata
    pub functions: Option<Vec<Function>>,

    /// Contract storage layout
    pub storage: Option<Vec<StorageRecord>>,
}

/// Builder for configuring contract analysis parameters
///
/// See [`contract_info`] for usage examples.
pub struct ContractInfoArgs<'a> {
    code: &'a [u8],

    need_selectors: bool,
    need_arguments: bool,
    need_state_mutability: bool,
    need_storage: bool,
}

impl<'a> ContractInfoArgs<'a> {
    /// Creates a new instance of contract analysis configuration
    ///
    /// # Arguments
    ///
    /// * `code` - A slice of deployed contract bytecode
    pub fn new(code: &'a [u8]) -> Self {
        ContractInfoArgs {
            code,
            need_selectors: false,
            need_arguments: false,
            need_state_mutability: false,
            need_storage: false,
        }
    }

    /// Enables the extraction of function selectors
    pub fn with_selectors(mut self) -> Self {
        self.need_selectors = true;
        self
    }

    /// Enables the extraction of function arguments
    pub fn with_arguments(mut self) -> Self {
        self.need_selectors = true;
        self.need_arguments = true;
        self
    }

    /// Enables the extraction of state mutability
    pub fn with_state_mutability(mut self) -> Self {
        self.need_selectors = true;
        self.need_state_mutability = true;
        self
    }

    /// Enables the extraction of the contract's storage layout
    pub fn with_storage(mut self) -> Self {
        self.need_selectors = true;
        self.need_arguments = true;
        self.need_storage = true;
        self
    }
}

/// Extracts information about a smart contract from its EVM bytecode.
///
/// # Parameters
///
/// - `args`: A [`ContractInfoArgs`] instance specifying what data to extract from the provided
///   bytecode. Use the builder-style methods on `ContractInfoArgs` (e.g., `.with_selectors()`,
///   `.with_arguments()`) to enable specific analyses.
///
/// # Returns
///
/// Returns a [`Contract`] object containing the requested smart contract information. The
/// `Contract` struct wraps optional fields depending on the configuration provided in `args`.
/// # Examples
///
/// ```
/// use evmole::{ContractInfoArgs, StateMutability, contract_info};
/// use alloy_primitives::hex;
///
/// let code = hex::decode("6080604052348015600e575f80fd5b50600436106030575f3560e01c80632125b65b146034578063b69ef8a8146044575b5f80fd5b6044603f3660046046565b505050565b005b5f805f606084860312156057575f80fd5b833563ffffffff811681146069575f80fd5b925060208401356001600160a01b03811681146083575f80fd5b915060408401356001600160e01b0381168114609d575f80fd5b80915050925092509256").unwrap();
///
/// // Extract function selectors and their state mutability
/// let args = ContractInfoArgs::new(&code)
///     .with_selectors()
///     .with_state_mutability();
///
/// let info = contract_info(args);
/// let fns = info.functions.unwrap();
/// assert_eq!(fns.len(), 2);
/// assert_eq!(fns[0].selector, [0x21, 0x25, 0xb6, 0x5b]);
/// assert_eq!(fns[0].state_mutability, Some(StateMutability::Pure));
/// ```
pub fn contract_info(args: ContractInfoArgs) -> Contract {
    const GAS_LIMIT: u32 = 0;

    let functions = if args.need_selectors {
        Some(
            function_selectors_with_pc(args.code, GAS_LIMIT)
                .into_iter()
                .map(|(selector, bytecode_offset)| Function {
                    selector,
                    arguments: if args.need_arguments {
                        #[allow(deprecated)]
                        Some(function_arguments_alloy(args.code, &selector, GAS_LIMIT))
                    } else {
                        None
                    },
                    state_mutability: if args.need_state_mutability {
                        #[allow(deprecated)]
                        Some(state_mutability::function_state_mutability(
                            args.code, &selector, GAS_LIMIT,
                        ))
                    } else {
                        None
                    },
                    bytecode_offset,
                })
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };

    //TODO: filter fns by state_mutability if available
    let storage = if args.need_storage {
        let fns = functions
            .as_ref()
            .expect("enabled on with_storage()")
            .iter()
            .map(|f| (f.selector, f.bytecode_offset, f.arguments.as_ref().unwrap()));
        Some(storage::contract_storage(args.code, fns, GAS_LIMIT))
    } else {
        None
    };

    Contract { functions, storage }
}
