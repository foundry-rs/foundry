//! Upper-bound gas accounting for symbolic paths.
//!
//! The meter charges every opcode before it executes, using revm's gas tables. When a cost
//! depends on a symbolic value, it charges the most expensive case or keeps the cost as a
//! symbolic term. When no sound bound is known, it records the reason and the snapshot
//! cheatcodes report the run as incomplete.

use super::*;
use foundry_evm::inspectors::cheatcodes::GAS_SNAPSHOT_OVERHEAD;

/// Position in the meter: the concrete gas and the number of symbolic terms.
pub(crate) type GasMark = (u64, usize);

/// Group and name of a gas snapshot.
pub(crate) type SnapshotName = (Option<String>, String);

/// Call depth that marks the root transaction in [`IsolatedCall`]. No opcode runs at it.
pub(crate) const TRANSACTION_DEPTH: usize = usize::MAX;

/// Name of the symbolic sender of the root transaction when its gas is measured.
pub(crate) const TRANSACTION_SENDER: &str = "transaction_sender";

#[derive(Clone, Debug, Default)]
pub(crate) struct GasMeter {
    /// Concrete part of the gas this path spent since it started.
    pub(crate) spent: u64,
    /// Symbolic part of the gas this path spent, added to `spent`.
    pub(crate) terms: Vec<SymExpr>,
    /// Position before the opcode that executes now.
    before_op: GasMark,
    /// Position and `(group, name)` of the active gas snapshot.
    pub(crate) snapshot: Option<(GasMark, SnapshotName)>,
    /// Snapshots this path measured, with their gas.
    pub(crate) measured: Vec<(SnapshotName, SymExpr)>,
    /// First reason the bound stopped being sound on this path.
    unmodeled: Option<&'static str>,
    /// `(call depth, pc)` of an opcode charged before the path forked inside it.
    charged: Option<(usize, usize)>,
    /// Accounts and slots this path accessed. Everything else is treated as cold.
    pub(crate) warm: WarmSet,
    /// Top-level call that runs as its own transaction, settled at the caller's next opcode.
    isolated: Option<Box<IsolatedCall>>,
    /// Storage writes on this path when the current transaction started, which give the
    /// original values `SSTORE` is priced against. `None` if they are unknown.
    pub(crate) transaction_writes: Option<usize>,
    /// Refunds of the isolated call's transaction. They can be negative, as two's complement.
    pub(crate) refunds: Vec<SymExpr>,
    /// Conditions under which the path reaches its calldata pricing, for the witness.
    pub(crate) witness_hints: Vec<SymBoolExpr>,
    /// Loops this path summarized instead of running every iteration.
    pub(crate) loops: Vec<SymbolicLoopBound>,
}

/// Accounts and storage slots a transaction accessed, for EIP-2929 pricing.
#[derive(Clone, Debug, Default)]
pub(crate) struct WarmSet {
    pub(crate) accounts: HashSet<Address>,
    /// Accounts with an unresolved address, keyed by the exact address expression.
    pub(crate) account_words: HashSet<SymExpr>,
    /// Slots keyed by the exact slot expression.
    pub(crate) slots: HashSet<(Address, SymExpr)>,
}

/// A top-level call that test isolation runs as a separate transaction.
///
/// The caller pays the transaction's receipt gas instead of the callee's execution gas. The
/// transaction has its own warm accounts and slots.
#[derive(Clone, Debug)]
struct IsolatedCall {
    call_depth: usize,
    /// Concrete gas spent when the callee started.
    start: u64,
    /// Transaction calldata, one expression per byte.
    input: Vec<SymExpr>,
    /// Caller's warm accounts and slots and transaction start, restored after the transaction.
    warm: WarmSet,
    transaction_writes: Option<usize>,
    refunds: Vec<SymExpr>,
}

impl GasMeter {
    /// Creates a meter for a transaction from `origin` to `target`, which EIP-2929 makes warm,
    /// that starts after `storage_writes` storage writes.
    pub(crate) fn new(origin: Address, target: Address, storage_writes: usize) -> Self {
        let mut meter = Self { transaction_writes: Some(storage_writes), ..Default::default() };
        meter.warm.accounts.extend([origin, target]);
        meter
    }

    /// Starts charging the opcode at `pc`. Returns `false` if this path already paid for it.
    ///
    /// A path that forks inside an opcode can resume at the same opcode. The marker stays set
    /// until the next opcode, and two consecutive opcodes never share a pc in one frame.
    pub(crate) fn begin_op(&mut self, call_depth: usize, pc: usize) -> bool {
        if self.charged.take() == Some((call_depth, pc)) {
            return false;
        }
        self.before_op = (self.spent, self.terms.len());
        self.charged = Some((call_depth, pc));
        true
    }

    pub(crate) const fn charge(&mut self, gas: u64) {
        self.spent = self.spent.saturating_add(gas);
    }

    /// Charges a cost that can depend on symbolic values.
    pub(crate) fn charge_expr(&mut self, gas: SymExpr) {
        match gas.as_const().and_then(|gas| u64::try_from(gas).ok()) {
            Some(gas) => self.charge(gas),
            None => self.terms.push(gas),
        }
    }

    pub(crate) fn unmodeled(&mut self, reason: &'static str) {
        self.unmodeled.get_or_insert(reason);
    }

    /// Records a refund if an isolated call is running. Only isolated calls pay receipt gas,
    /// which subtracts refunds.
    pub(crate) fn refund(&mut self, refund: SymExpr) {
        if self.isolated.is_some() {
            self.refunds.push(refund);
        }
    }

    /// Returns the gas spent after `mark`, or the reason it has no sound bound.
    pub(crate) fn since(&self, cx: &mut SymCx, mark: GasMark) -> Result<SymExpr, SymbolicError> {
        if let Some(reason) = self.unmodeled {
            return Err(SymbolicError::Unsupported(reason));
        }
        let (spent, terms) = mark;
        let spent = SymExpr::constant(cx, U256::from(self.spent - spent));
        Ok(self.terms[terms..]
            .iter()
            .fold(spent, |total, term| SymExpr::binop(cx, SymBinOp::Add, total, term.clone())))
    }

    /// Starts a top-level call to `target` that runs as its own transaction with `input`.
    ///
    /// The transaction's sender is treated as cold, since a prank can change it.
    pub(crate) fn begin_isolated_call(
        &mut self,
        call_depth: usize,
        target: Option<Address>,
        input: Vec<SymExpr>,
        storage_writes: usize,
    ) {
        let warm = WarmSet { accounts: HashSet::from_iter(target), ..Default::default() };
        let warm = std::mem::replace(&mut self.warm, warm);
        let transaction_writes = self.transaction_writes.replace(storage_writes);
        let refunds = std::mem::take(&mut self.refunds);
        self.isolated = Some(Box::new(IsolatedCall {
            call_depth,
            start: self.spent,
            input,
            warm,
            transaction_writes,
            refunds,
        }));
    }

    /// Measures the gas of the whole path as the snapshot `name`.
    pub(crate) fn measure_path(
        &mut self,
        cx: &mut SymCx,
        name: SnapshotName,
    ) -> Result<(), SymbolicError> {
        if let Some(reason) = self.unmodeled {
            return Err(SymbolicError::Unsupported(reason));
        }
        let spent = SymExpr::constant(cx, U256::from(self.spent));
        let gas = self
            .terms
            .iter()
            .fold(spent, |total, term| SymExpr::binop(cx, SymBinOp::Add, total, term.clone()));
        self.measured.push((name, gas));
        Ok(())
    }

    /// Takes the isolated call that returned to the opcode at `pc`, if any.
    ///
    /// Restores the caller's state and returns the transaction calldata, the callee's execution
    /// gas, and its refunds. The caller then pays the receipt gas with
    /// [`Self::settle_isolated_call`].
    pub(crate) fn returned_isolated_call(
        &mut self,
        call_depth: usize,
        pc: usize,
    ) -> Option<(Vec<SymExpr>, u64, Vec<SymExpr>)> {
        if self.charged == Some((call_depth, pc)) {
            return None;
        }
        let call = self.isolated.take_if(|call| call.call_depth == call_depth)?;
        let execution = self.spent - call.start;
        self.spent = call.start;
        self.warm = call.warm;
        self.transaction_writes = call.transaction_writes;
        let refunds = std::mem::replace(&mut self.refunds, call.refunds);
        Some((call.input, execution, refunds))
    }

    /// Charges the receipt gas of an isolated call, as revm computes it after execution: the
    /// intrinsic gas plus `execution`, minus `refunds` capped at `max_refund`, and at least the
    /// EIP-7623 `floor`.
    ///
    /// The caller passes the EIP-3529 cap for the cheapest calldata. It is at most the real cap,
    /// so the receipt stays an upper bound, and it avoids a symbolic division.
    pub(crate) fn settle_isolated_call(
        &mut self,
        cx: &mut SymCx,
        intrinsic: SymExpr,
        floor: SymExpr,
        execution: u64,
        refunds: Vec<SymExpr>,
        max_refund: u64,
    ) {
        let execution = SymExpr::constant(cx, U256::from(execution));
        let used = SymExpr::binop(cx, SymBinOp::Add, intrinsic, execution);
        let zero = SymExpr::zero(cx);
        let refund = refunds
            .into_iter()
            .fold(zero, |total, refund| SymExpr::binop(cx, SymBinOp::Add, total, refund));
        let max_refund = SymExpr::constant(cx, U256::from(max_refund));
        let capped = SymBoolExpr::cmp_word_expr(cx, SymCmpOp::Ult, &refund, max_refund.clone());
        let refund = SymExpr::ite(cx, capped, refund, max_refund);
        let used = SymExpr::binop(cx, SymBinOp::Sub, used, refund);
        let below_floor = SymBoolExpr::cmp_word_expr(cx, SymCmpOp::Ult, &used, floor.clone());
        let receipt = SymExpr::ite(cx, below_floor, floor, used);
        self.charge_expr(receipt);
    }

    /// Starts a gas snapshot.
    ///
    /// Gas spent before the snapshot does not change the gas inside it, so an unmodeled cost
    /// before this point is forgotten. Warm accounts and slots stay tracked.
    pub(crate) fn start_snapshot(&mut self, name: SnapshotName) {
        self.snapshot = Some(((self.spent, self.terms.len()), name));
        self.unmodeled = None;
    }

    /// Ends the active snapshot and returns the gas spent inside it, before the current opcode.
    pub(crate) fn stop_snapshot(&mut self, cx: &mut SymCx) -> Result<SymExpr, SymbolicError> {
        let ((start, start_terms), name) = self
            .snapshot
            .take()
            .ok_or(SymbolicError::Unsupported("symbolic vm.stopSnapshotGas without snapshot"))?;
        if let Some(reason) = self.unmodeled {
            return Err(SymbolicError::Unsupported(reason));
        }
        let (end, end_terms) = self.before_op;
        let spent = end.saturating_sub(start).saturating_sub(GAS_SNAPSHOT_OVERHEAD);
        let spent = SymExpr::constant(cx, U256::from(spent));
        let gas = self.terms[start_terms..end_terms]
            .iter()
            .fold(spent, |total, term| SymExpr::binop(cx, SymBinOp::Add, total, term.clone()));
        self.measured.push((name, gas.clone()));
        Ok(gas)
    }

    /// Whether the gas spent now is measured, by a snapshot or as the root transaction.
    ///
    /// Outside a measurement only warm accounts and slots matter, so costs that need solver
    /// queries or storage reads are charged at their maximum instead.
    pub(crate) fn is_measuring(&self) -> bool {
        self.snapshot.is_some()
            || self.isolated.as_ref().is_some_and(|call| call.call_depth == TRANSACTION_DEPTH)
    }

    /// Joins a finished child call into its parent.
    ///
    /// The parent pays the gas the child spent. Warm accesses survive only a successful child:
    /// EIP-2929 reverts them with the rest of the child's state.
    pub(crate) fn join_child(&mut self, child: &mut Self, success: bool) {
        self.spent = child.spent;
        self.terms = std::mem::take(&mut child.terms);
        self.before_op = child.before_op;
        self.snapshot = child.snapshot.take();
        self.measured = std::mem::take(&mut child.measured);
        self.witness_hints = std::mem::take(&mut child.witness_hints);
        self.unmodeled = child.unmodeled;
        if success {
            self.refunds = std::mem::take(&mut child.refunds);
            self.warm = std::mem::take(&mut child.warm);
        }
    }
}
