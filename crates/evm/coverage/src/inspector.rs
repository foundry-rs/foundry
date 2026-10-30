use crate::{CallData, HitMap, HitMaps};
use alloy_primitives::B256;
use evm2::{
    EvmTypesHost, Inspector,
    interpreter::{Interpreter, Message, MessageResult},
};
use std::ptr::NonNull;

/// Inspector implementation for collecting coverage information.
#[derive(Clone, Debug)]
pub struct LineCoverageCollector {
    // NOTE: `current_map` is always a valid reference into `maps`.
    // It is accessed only through `get_or_insert_map` which guarantees that it's valid.
    // Both of these fields are unsafe to access directly outside of `*insert_map`.
    current_map: NonNull<HitMap>,
    current_hash: B256,

    maps: HitMaps,
}

// SAFETY: See comments on `current_map`.
unsafe impl Send for LineCoverageCollector {}
unsafe impl Sync for LineCoverageCollector {}

impl Default for LineCoverageCollector {
    fn default() -> Self {
        Self {
            current_map: NonNull::dangling(),
            current_hash: B256::ZERO,
            maps: Default::default(),
        }
    }
}

impl<T: EvmTypesHost> Inspector<T> for LineCoverageCollector {
    fn initialize_interp(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        let message = interp.message();
        let call = (!message.kind.is_create())
            .then(|| (CallData::new(&message.input), !message.value.is_zero()));
        let map = self.get_or_insert_map(interp);
        if let Some((call, with_value)) = call {
            map.call(call, with_value);
        }
    }

    fn step(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        let pc = interp.pc();
        self.get_or_insert_map(interp).record(pc);
    }

    fn create_end(
        &mut self,
        _interp: &mut Interpreter<'_, '_, T>,
        message: &Message<T>,
        result: &mut MessageResult<T>,
    ) {
        if result.stop.is_success()
            && let Some(map) = self.maps.get_mut(&message.code.hash_slow())
        {
            map.creation();
        }
    }
}

impl LineCoverageCollector {
    /// Finish collecting coverage information and return the [`HitMaps`].
    pub fn finish(mut self) -> HitMaps {
        self.maps.values_mut().for_each(HitMap::flush);
        self.maps
    }

    /// Gets the hit map for the current contract, or inserts a new one if it doesn't exist.
    ///
    /// The map is stored in `current_map` and returned as a mutable reference.
    /// See comments on `current_map` for more details.
    #[inline]
    fn get_or_insert_map<T: EvmTypesHost>(
        &mut self,
        interp: &Interpreter<'_, '_, T>,
    ) -> &mut HitMap {
        let hash = interp.original_bytecode_hash();
        if self.current_hash != hash {
            self.insert_map(hash, interp);
        }
        // SAFETY: See comments on `current_map`.
        unsafe { self.current_map.as_mut() }
    }

    #[cold]
    #[inline(never)]
    fn insert_map<T: EvmTypesHost>(&mut self, hash: B256, interp: &Interpreter<'_, '_, T>) {
        self.current_hash = hash;
        // Converts the mutable reference to a `NonNull` pointer.
        self.current_map =
            self.maps.entry(hash).or_insert_with(|| HitMap::new(interp.original_bytecode())).into();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use alloy_network::TransactionBuilder;
    use alloy_primitives::{Address, Bytes, U256, keccak256};
    use evm2::{
        bytecode::Bytecode,
        evm::{AccountInfo, InMemoryDB},
    };
    use foundry_evm_core::evm::{
        EvmEnv, EvmFactoryFor, FoundryEvmFactory, FoundryEvmNetwork, SpecFor,
        TransactionRequestFor, TypesFor,
    };

    pub(crate) const TARGET: Address = Address::with_last_byte(0xC0);

    /// `if (calldataload(0) == 1) { jump(10) }; stop`, with a `JUMPDEST; STOP` at pc 10.
    pub(crate) const BRANCH: &[u8] =
        &[0x60, 0x00, 0x35, 0x60, 0x01, 0x14, 0x60, 0x0a, 0x57, 0x00, 0x5b, 0x00];

    /// Calls `TARGET` running `code` with `input` on `FEN`'s default spec, returning the
    /// transaction status and the inspector.
    pub(crate) fn inspect<FEN: FoundryEvmNetwork, I: Inspector<TypesFor<FEN>> + 'static>(
        code: &[u8],
        input: U256,
        inspector: I,
    ) -> (bool, I)
    where
        TransactionRequestFor<FEN>: Default,
    {
        let caller = Address::with_last_byte(0xCA);
        let chain_id = 31337;
        let mut db = InMemoryDB::default();
        db.insert_account_info(&caller, AccountInfo::default().with_balance(U256::from(1e18)));
        db.insert_account_info(
            &TARGET,
            AccountInfo::default().with_code(Bytecode::new_legacy(Bytes::copy_from_slice(code))),
        );
        let factory = EvmFactoryFor::<FEN>::default();
        let spec = SpecFor::<FEN>::default();
        let env = EvmEnv::new(spec, factory.version(spec, chain_id), Default::default());
        let mut evm = factory.create_evm(db, env);
        evm.set_inspector(inspector);
        let request = TransactionRequestFor::<FEN>::default()
            .with_from(caller)
            .with_to(TARGET)
            .with_input(input.to_be_bytes::<32>())
            .with_nonce(0)
            .with_chain_id(chain_id)
            .with_gas_limit(1_000_000)
            .with_max_fee_per_gas(0)
            .with_max_priority_fee_per_gas(0);
        let tx = FEN::tx_from_request(request).unwrap();
        let status = evm.transact(&tx).unwrap().commit().status;
        (status, *evm.clear_inspector_as::<I>().unwrap())
    }

    fn line_coverage_suite<FEN: FoundryEvmNetwork>()
    where
        TransactionRequestFor<FEN>: Default,
    {
        for (input, pcs) in [(1, &[0, 2, 3, 5, 6, 8, 10, 11][..]), (2, &[0, 2, 3, 5, 6, 8, 9][..])]
        {
            let (status, collector) =
                inspect::<FEN, _>(BRANCH, U256::from(input), LineCoverageCollector::default());
            assert!(status);
            let maps = collector.finish();
            let map = &maps[&keccak256(BRANCH)];
            let mut hits = map.iter().collect::<Vec<_>>();
            hits.sort_unstable();
            assert_eq!(hits, pcs.iter().map(|&pc| (pc, 1)).collect::<Vec<_>>());
            // The small inputs are left-padded, so their selector is zero.
            assert_eq!(map.selector_calls[&[0; 4]].without_value, 1);
            assert_eq!(map.creations, 0);
        }

        // A successful `create(0, 0, 1)` of the init code `STOP` counts one creation.
        let (status, collector) = inspect::<FEN, _>(
            &[0x60, 0x01, 0x60, 0x00, 0x60, 0x00, 0xf0, 0x00],
            U256::ZERO,
            LineCoverageCollector::default(),
        );
        assert!(status);
        assert_eq!(collector.finish()[&keccak256([0x00])].creations, 1);
    }

    #[test]
    fn line_coverage() {
        line_coverage_suite::<foundry_evm_core::evm::EthEvmNetwork>();
        line_coverage_suite::<foundry_evm_core::evm::TempoEvmNetwork>();
        #[cfg(feature = "optimism")]
        line_coverage_suite::<foundry_evm_core::evm::OpEvmNetwork>();
        #[cfg(feature = "base")]
        line_coverage_suite::<foundry_evm_core::evm::BaseEvmNetwork>();
        #[cfg(feature = "monad")]
        line_coverage_suite::<foundry_evm_core::evm::MonadEvmNetwork>();
    }
}
