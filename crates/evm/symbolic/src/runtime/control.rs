use super::*;

#[derive(Clone, Copy, Debug)]
pub(crate) enum ShiftKind {
    Shl,
    Shr,
    Sar,
}

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
    Failure,
    AssumeRejected,
}

pub(crate) enum CheatcodeOutcome {
    Continue(Vec<SymWord>),
    ContinueData(SymReturnData),
    AssumeRejected,
    Failure,
}
