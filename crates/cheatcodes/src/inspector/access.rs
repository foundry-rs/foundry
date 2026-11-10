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
        // revm resolves delegated bytecode but retains the EOA as bytecode_address. The account's
        // code is already loaded, so resolving the designator here does not warm another account.
        let code = interpreter.input.bytecode_address.map_or(context, |address| {
            ecx.journal()
                .evm_state()
                .get(&address)
                .and_then(|account| account.info.code.as_ref())
                .and_then(|code| code.eip7702_address())
                .unwrap_or(address)
        });
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
        if !db.is_forked_mode() {
            return Ok(());
        }
        db.ensure_cheatcode_access(&caller)?;
        if let Some(frame) = self.frame_access.last() {
            if frame.context != caller {
                db.ensure_cheatcode_access(&frame.context)?;
            }
            if frame.code != caller && frame.code != frame.context {
                db.ensure_cheatcode_access(&frame.code)?;
            }
        }
        Ok(())
    }

    pub(super) fn can_inherit_cheatcode_access(&self, ecx: &FoundryContextFor<'_, FEN>) -> bool {
        let db = ecx.db();
        if !db.is_forked_mode() {
            return true;
        }
        let Some((creator, ancestors)) = self.frame_access.split_last() else {
            return true;
        };

        // Trusted callbacks may grant children access despite untrusted callers. Untrusted code
        // borrowing an authorized context must not gain access through an authorized factory.
        db.has_cheatcode_access(&creator.context)
            && (creator.code == creator.context || db.has_cheatcode_access(&creator.code))
            && ancestors.iter().all(|frame| {
                frame.code == frame.context
                    || !db.has_cheatcode_access(&frame.context)
                    || db.has_cheatcode_access(&frame.code)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(depth: usize) -> FrameAccess {
        FrameAccess {
            depth,
            context: Address::with_last_byte(depth as u8),
            code: Address::with_last_byte(depth as u8 + 1),
        }
    }

    fn identities(frames: &[FrameAccess]) -> Vec<(usize, Address, Address)> {
        frames.iter().map(|frame| (frame.depth, frame.context, frame.code)).collect()
    }

    #[test]
    fn mixed_nested_frame_access_preserves_each_suspended_parent() {
        for inherited in [false, true] {
            let mut cheats = Cheatcodes::default();
            cheats.frame_access.push(frame(1));
            let parent = identities(&cheats.frame_access);
            let nested = |cheats: &mut Cheatcodes| {
                if !inherited {
                    assert!(cheats.frame_access.is_empty());
                }
                cheats.frame_access.push(frame(2));
                let suspended = identities(&cheats.frame_access);
                let clear = |cheats: &mut Cheatcodes| {
                    if inherited {
                        assert!(cheats.frame_access.is_empty());
                    } else {
                        assert_eq!(identities(&cheats.frame_access), suspended);
                    }
                    cheats.frame_access.clear();
                    cheats.frame_access.push(frame(3));
                    Err::<(), _>("expected inner failure")
                };
                let result = if inherited {
                    cheats.with_fresh_frame_access(clear)
                } else {
                    cheats.with_nested_frame_access(clear)
                };
                assert_eq!(result, Err("expected inner failure"));
                assert_eq!(identities(&cheats.frame_access), suspended);
                cheats.frame_access.clear();
                result
            };
            let result = if inherited {
                cheats.with_nested_frame_access(nested)
            } else {
                cheats.with_fresh_frame_access(nested)
            };
            assert_eq!(result, Err("expected inner failure"));
            assert_eq!(identities(&cheats.frame_access), parent);
        }
    }
}
