//! Authorization for the context and code executing each interpreter frame.

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FrameAccess {
    depth: u16,
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

    pub(super) fn enter_frame_access(&mut self, interp: &InterpreterFor<'_, '_, FEN>) {
        let message = interp.message();
        let depth = message.depth;
        while self.frame_access.last().is_some_and(|frame| frame.depth >= depth) {
            self.frame_access.pop();
        }
        let context = message.destination;
        // Constructors execute initcode supplied by the creator, under the newly granted address.
        // The message already identifies the resolved code for delegated EIP-7702 execution.
        let code = if message.kind.is_create() { context } else { message.code_address };
        self.frame_access.push(FrameAccess { depth, context, code });
    }

    /// Removes the ended frame and any descendants left behind by aborted nested execution.
    pub fn leave_frame_access(&mut self, depth: u16) {
        while self.frame_access.last().is_some_and(|frame| frame.depth >= depth) {
            self.frame_access.pop();
        }
    }

    pub(super) fn ensure_cheatcode_access(
        &self,
        backend: &dyn CheatcodesBackend<FEN>,
        caller: Address,
    ) -> Result<()> {
        let db = backend.backend();
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

    pub(super) fn can_inherit_cheatcode_access(
        &self,
        backend: &dyn CheatcodesBackend<FEN>,
    ) -> bool {
        let db = backend.backend();
        if !db.is_forked_mode() {
            return true;
        }

        // Trusted callbacks may grant children access despite untrusted callers. Untrusted code
        // borrowing an authorized context must not gain access through an authorized factory.
        self.frame_access.last().is_none_or(|creator| db.has_cheatcode_access(&creator.context))
            && self.frame_access.iter().all(|frame| {
                frame.code == frame.context
                    || !db.has_cheatcode_access(&frame.context)
                    || db.has_cheatcode_access(&frame.code)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(depth: u16) -> FrameAccess {
        FrameAccess {
            depth,
            context: Address::with_last_byte(depth as u8),
            code: Address::with_last_byte(depth as u8 + 1),
        }
    }

    #[test]
    fn mixed_nested_frame_access_preserves_each_suspended_parent() {
        for inherited in [false, true] {
            let mut cheats = Cheatcodes::default();
            cheats.frame_access.push(frame(1));
            let parent = cheats.frame_access.clone();
            let nested = |cheats: &mut Cheatcodes| {
                if !inherited {
                    assert!(cheats.frame_access.is_empty());
                }
                cheats.frame_access.push(frame(2));
                let suspended = cheats.frame_access.clone();
                let clear = |cheats: &mut Cheatcodes| {
                    if inherited {
                        assert!(cheats.frame_access.is_empty());
                    } else {
                        assert_eq!(cheats.frame_access, suspended);
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
                assert_eq!(cheats.frame_access, suspended);
                cheats.frame_access.clear();
                result
            };
            let result = if inherited {
                cheats.with_nested_frame_access(nested)
            } else {
                cheats.with_fresh_frame_access(nested)
            };
            assert_eq!(result, Err("expected inner failure"));
            assert_eq!(cheats.frame_access, parent);
        }
    }

    #[test]
    fn ended_frame_access_discards_descendants_and_preserves_parent() {
        let mut cheats = Cheatcodes::<EthEvmNetwork> {
            frame_access: (0..=3).map(frame).collect(),
            ..Default::default()
        };

        cheats.leave_frame_access(2);
        assert_eq!(cheats.frame_access, [frame(0), frame(1)]);
        cheats.leave_frame_access(1);
        assert_eq!(cheats.frame_access, [frame(0)]);
        cheats.leave_frame_access(0);
        assert!(cheats.frame_access.is_empty());
    }
}
