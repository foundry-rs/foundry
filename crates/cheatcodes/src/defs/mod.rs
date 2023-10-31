//! Cheatcode definitions.

use serde::{Deserialize, Serialize};
use std::{borrow::Cow, fmt};

mod cheatcode;
pub use cheatcode::{Cheatcode, CheatcodeDef, Group, Safety, Status};

mod function;
pub use function::{Function, Mutability, Visibility};

mod items;
pub use items::{Enum, EnumVariant, Error, Event, Struct, StructField};

mod vm;
pub use vm::Vm;

// The `cheatcodes.json` schema.
/// Foundry cheatcodes. Learn more: <https://book.getfoundry.sh/cheatcodes/>
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Cheatcodes<'a> {
    /// Cheatcode errors.
    #[serde(borrow)]
    pub errors: Cow<'a, [Error<'a>]>,
    /// Cheatcode events.
    #[serde(borrow)]
    pub events: Cow<'a, [Event<'a>]>,
    /// Cheatcode enums.
    #[serde(borrow)]
    pub enums: Cow<'a, [Enum<'a>]>,
    /// Cheatcode structs.
    #[serde(borrow)]
    pub structs: Cow<'a, [Struct<'a>]>,
    /// All the cheatcodes.
    #[serde(borrow)]
    pub cheatcodes: Cow<'a, [Cheatcode<'a>]>,
}

impl fmt::Display for Cheatcodes<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for error in self.errors.iter() {
            writeln!(f, "{error}")?;
        }
        for event in self.events.iter() {
            writeln!(f, "{event}")?;
        }
        for enumm in self.enums.iter() {
            writeln!(f, "{enumm}")?;
        }
        for strukt in self.structs.iter() {
            writeln!(f, "{strukt}")?;
        }
        for cheatcode in self.cheatcodes.iter() {
            writeln!(f, "{}", cheatcode.func)?;
        }
        Ok(())
    }
}

impl Cheatcodes<'static> {
    /// Returns the default cheatcodes.
    pub fn new() -> Self {
        Self {
            // unfortunately technology has not yet advanced to the point where we can get all
            // items of a certain type in a module, so we have to hardcode them here
            structs: Cow::Owned(vec![
                Vm::Log::STRUCT.clone(),
                Vm::Rpc::STRUCT.clone(),
                Vm::EthGetLogs::STRUCT.clone(),
                Vm::DirEntry::STRUCT.clone(),
                Vm::FsMetadata::STRUCT.clone(),
                Vm::Wallet::STRUCT.clone(),
                Vm::FfiResult::STRUCT.clone(),
            ]),
            enums: Cow::Owned(vec![Vm::CallerMode::ENUM.clone()]),
            errors: Vm::VM_ERRORS.iter().map(|&x| x.clone()).collect(),
            events: Cow::Borrowed(&[]),
            // events: Vm::VM_EVENTS.iter().map(|&x| x.clone()).collect(),
            cheatcodes: Vm::CHEATCODES.iter().map(|&x| x.clone()).collect(),
        }
    }
}
