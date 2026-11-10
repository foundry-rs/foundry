//! Authorization for the context and code executing each interpreter frame.

use super::*;

#[derive(Clone, Debug)]
pub(super) struct FrameAccess {
    depth: usize,
    context: Address,
    code: Address,
}

impl<FEN: FoundryEvmNetwork> Cheatcodes<FEN> {
    /// Gives an independent nested execution its own frame identities.
    pub fn with_fresh_frame_access<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let frames = std::mem::take(&mut self.frame_access);
        let result = f(self);
        self.frame_access = frames;
        result
    }

    /// Preserves the suspended caller when inherited execution aborts before frame cleanup.
    pub fn with_nested_frame_access<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let frames = self.frame_access.clone();
        let result = f(self);
        self.frame_access = frames;
        result
    }

    pub(super) fn enter_frame_access(
        &mut self,
        interpreter: &Interpreter,
        ecx: &FoundryContextFor<'_, FEN>,
    ) {
        let depth = ecx.journal().depth();
        while self.frame_access.last().is_some_and(|frame| frame.depth >= depth) {
            self.frame_access.pop();
        }
        let context = interpreter.input.target_address;
        // Constructors execute initcode supplied by the creator, under the newly granted address.
        let nominal_code = interpreter.input.bytecode_address.unwrap_or(context);
        // revm resolves delegated bytecode but retains the EOA as bytecode_address. The account's
        // code is already loaded, so resolving the designator here does not warm another account.
        let code = ecx
            .journal()
            .evm_state()
            .get(&nominal_code)
            .and_then(|account| account.info.code.as_ref())
            .and_then(|code| code.eip7702_address())
            .unwrap_or(nominal_code);
        self.frame_access.push(FrameAccess { depth, context, code });
    }

    pub(super) fn leave_frame_access(&mut self, depth: usize) {
        while self.frame_access.last().is_some_and(|frame| frame.depth > depth) {
            self.frame_access.pop();
        }
    }

    pub(super) fn ensure_cheatcode_access(
        &self,
        ecx: &FoundryContextFor<'_, FEN>,
        caller: Address,
    ) -> Result<()> {
        let db = ecx.db();
        db.ensure_cheatcode_access_forking_mode(&caller)?;
        if db.is_forked_mode()
            && let Some(frame) = self.frame_access.last()
        {
            db.ensure_cheatcode_access(&frame.context)?;
            db.ensure_cheatcode_access(&frame.code)?;
        }
        Ok(())
    }

    pub(super) fn can_inherit_cheatcode_access(&self, ecx: &FoundryContextFor<'_, FEN>) -> bool {
        // A generic factory must not turn an untrusted ancestor's initcode into privileged code.
        !ecx.db().is_forked_mode()
            || self.frame_access.iter().all(|frame| {
                ecx.db().has_cheatcode_access(&frame.context)
                    && ecx.db().has_cheatcode_access(&frame.code)
            })
    }
}
