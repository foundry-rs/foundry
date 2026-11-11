use super::*;

#[derive(Clone, Copy, Debug)]
pub(crate) enum CallKind {
    Call,
    CallCode,
    DelegateCall,
    StaticCall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CreateKind {
    Create,
    Create2,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum StepOutcome {
    Continue,
    Forked,
    Halt,
    Revert,
    ExceptionalHalt,
    Failure,
    AssumeRejected,
}

pub(crate) enum CheatcodeOutcome {
    Continue(Vec<SymExpr>),
    ContinueData(SymReturnData),
    Revert(SymReturnData),
    /// A failed assertion that may also pass: `passing` continues as a successful call, and the
    /// current path fails the assertion, reverting with `revert` or setting the fail slot.
    Fork {
        passing: Box<PathState>,
        revert: Option<SymReturnData>,
    },
    /// A call that returns one of several values: each choice continues on its own path with its
    /// constraints. The list is never empty.
    Choices(Vec<(Vec<SymBoolExpr>, SymReturnData)>),
    Failure,
}
