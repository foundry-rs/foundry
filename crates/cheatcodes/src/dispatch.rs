//! Engine-independent cheatcode dispatch metadata.

use crate::Vm;

/// Returns the definition for every generated cheatcode call.
pub(crate) const fn metadata(calls: &Vm::VmCalls) -> &'static spec::Cheatcode<'static> {
    macro_rules! get_cheatcode {
        ($($variant:ident),*) => {
            match calls {
                $(Vm::VmCalls::$variant(cheat) => cheatcode_of(cheat),)*
            }
        };
    }

    vm_calls!(get_cheatcode)
}

const fn cheatcode_of<T: spec::CheatcodeDef>(_: &T) -> &'static spec::Cheatcode<'static> {
    T::CHEATCODE
}

pub(crate) fn name(cheat: &spec::Cheatcode<'static>) -> &'static str {
    cheat.func.signature.split('(').next().unwrap()
}

pub(crate) const fn id(cheat: &spec::Cheatcode<'static>) -> &'static str {
    cheat.func.id
}

pub(crate) const fn signature(cheat: &spec::Cheatcode<'static>) -> &'static str {
    cheat.func.signature
}
