//! Filesystem manipulation operations for solidity.

use crate::sputnik::script::handler::ScriptStackExecutor;
use sputnik::{backend::Backend, executor::stack::PrecompileSet};
use std::{collections::HashMap, fs::File};

impl<'a, 'b, Back: Backend, Pre: PrecompileSet + 'b> ScriptStackExecutor<'a, 'b, Back, Pre> {
    fn on_fs_call(&mut self) {}
}

/// Manages the state of the solidity `Fs` lib
#[derive(Debug, Default)]
pub struct FsManager {
    /// tracks all open files
    files: HashMap<usize, File>,
    /// counter used to determine the next file id
    file_ctn: usize,
}

ethers::contract::abigen!(
    ForgeFs,
    r#"[
            struct File { uint256 id; string path;}
            create(string)(File)
            write(File, string)(uint256)
    ]"#,
);
