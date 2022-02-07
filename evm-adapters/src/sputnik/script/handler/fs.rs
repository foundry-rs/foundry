//! Filesystem manipulation operations for solidity.

use crate::sputnik::{
    script::handler::ScriptStackExecutor,
    utils::{self, EvmCallResponse},
};
use ethers::abi::AbiEncode;
use ethers_core::types::H160;
use sputnik::{backend::Backend, executor::stack::PrecompileSet};
use std::{collections::HashMap, fs::File as StdFile};

impl<'a, 'b, Back: Backend, Pre: PrecompileSet + 'b> ScriptStackExecutor<'a, 'b, Back, Pre> {
    /// The callback invoked if a `fs` related call was made
    pub(crate) fn on_fs_call(&mut self, call: ForgeFsCalls, _caller: H160) -> EvmCallResponse {
        println!("received fs call: {:?}", call);
        match call {
            ForgeFsCalls::Create(call) => self.state.fs.create(call.path),
            ForgeFsCalls::Write(call) => self.state.fs.write_to_file(call.file, call.content),
        }
    }
}

/// Manages the state of the solidity `Fs` lib
#[derive(Debug, Default)]
pub struct FsManager {
    /// tracks all open files
    open_files: HashMap<usize, (StdFile, String)>,
    /// counter used to determine the next file id
    file_ctn: usize,
}

impl FsManager {
    fn next_file_id(&mut self) -> usize {
        let id = self.file_ctn;
        self.file_ctn += 1;
        id
    }

    /// Creates the File and returns the `File` as response
    fn create(&mut self, path: String) -> EvmCallResponse {
        utils::try_respond(|| {
            let file = std::fs::File::create(&path)?;
            let file_id = self.next_file_id();
            self.open_files.insert(file_id, (file, path.clone()));

            let file = File { id: file_id.into(), path };

            Ok(file.encode())
        })
    }

    /// Writes the content to the file
    fn write_to_file(&mut self, file: File, content: String) -> EvmCallResponse {
        self.write(file.path, content)
    }
    /// Writes the content to the file
    fn write(&mut self, path: String, content: String) -> EvmCallResponse {
        utils::try_respond(|| {
            std::fs::write(path, content)?;
            Ok(vec![])
        })
    }
}

ethers::contract::abigen!(
    ForgeFs,
    r#"[
            struct File { uint256 id; string path;}
            create(string path)(File)
            write(File file, string content)
    ]"#,
);

#[cfg(test)]
mod tests {
    use crate::{sputnik::script::helpers::script_vm, Evm};
    use ethers_core::types::Address;
    use ethers_solc::{project_util::TempProject, ProjectPathsConfig};
    use std::path::PathBuf;

    fn new_project() -> TempProject {
        let script_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("script-lib");
        TempProject::new(
            ProjectPathsConfig::builder()
                .remapping(format!("forge-core/={}/", script_dir.display()).parse().unwrap())
                .lib(script_dir),
        )
        .unwrap()
    }

    #[test]
    fn can_create_file() {
        let mut evm = script_vm();

        let project = new_project();

        let outfile = project.paths().artifacts.join("output.txt");
        let content = "hello world!";

        project
            .add_source(
                "Demo",
                format!(
                    r##"
pragma solidity >=0.8.0 <0.9.0;
pragma experimental ABIEncoderV2;

import "forge-core/Fs.sol";


contract Demo {{
      Fs constant fs = Fs(FORGE_SCRIPT_ADDRESS);

      function run() external {{
        File memory file = fs.create("{}");
        fs.write(file, "{}");
      }}
}}"##,
                    outfile.display(),
                    content
                ),
            )
            .unwrap();

        let compiled = project.compile().unwrap();
        let output = compiled.output();
        let contract = output.find("Demo").expect("could not find contract").clone();

        let (addr, _, _, _) =
            evm.deploy(Address::zero(), contract.bytecode().unwrap().clone(), 0u64.into()).unwrap();

        let (_, reason, _, _) = evm
            .call::<(), _, _>(Address::zero(), addr, "run()", (), 0u64.into(), contract.abi)
            .unwrap();

        assert!(reason.is_succeed());
        assert_eq!(std::fs::read_to_string(outfile).unwrap(), content);
    }
}
