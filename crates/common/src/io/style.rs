#![allow(missing_docs)]
use anstyle::*;

pub const ERROR: Style = AnsiColor::Red.on_default().effects(Effects::BOLD);
pub const ERROR_MESSAGE: Style = AnsiColor::Red.on_default();
pub const WARN: Style = AnsiColor::Yellow.on_default().effects(Effects::BOLD);
pub const WARN_MESSAGE: Style = AnsiColor::Yellow.on_default();
