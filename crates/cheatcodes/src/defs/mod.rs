//! Cheatcode definitions.

use serde::{Deserialize, Serialize};
use std::borrow::Cow;

mod cheatcode;
pub use cheatcode::{Cheatcode, CheatcodeDef, Group, Safety, Status};

mod function;
pub use function::{Function, Mutability, Visibility};

mod items;
pub use items::{Enum, EnumVariant, Error, Event, Struct, StructField};

mod vm;
pub use vm::Vm;
#[cfg(test)]
pub(crate) use vm::VM_IFACE;

// The `cheatcodes.json` schema.
/// Foundry cheatcodes. Learn more: <https://book.getfoundry.sh/cheatcodes/>
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Cheatcodes<'a> {
    /// Cheatcode structs.
    #[serde(borrow)]
    pub structs: Cow<'a, [Struct<'a>]>,
    /// Cheatcode enums.
    #[serde(borrow)]
    pub enums: Cow<'a, [Enum<'a>]>,
    /// Cheatcode errors.
    #[serde(borrow)]
    pub errors: Cow<'a, [Error<'a>]>,
    /// Cheatcode events.
    #[serde(borrow)]
    pub events: Cow<'a, [Event<'a>]>,
    /// All the cheatcodes.
    #[serde(borrow)]
    pub cheatcodes: Cow<'a, [Cheatcode<'a>]>,
}

impl Cheatcodes<'static> {
    /// Returns the default cheatcodes.
    pub fn new() -> Self {
        Self {
            structs: Cow::Borrowed(&[]),
            enums: Cow::Borrowed(&[]),
            errors: Cow::Borrowed(&[]),
            events: Cow::Borrowed(&[]),
            cheatcodes: Vm::CHEATCODES.iter().map(|&x| x.clone()).collect(),
        }
    }
}
