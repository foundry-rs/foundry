// Generate mutants then run tests (reuse the whole unit test flow for now, including compilation to select mutants)
// Use Solar: 
use solar_parse::{
    token::{Token, TokenKind},
    Lexer,
    ast::{
        interface::{self, Session},
        Arena, CommentKind, Item, ItemKind,
        SourceUnit
    },
    Parser,
};
use std::sync::Arc;

use std::path::PathBuf;

struct MutationType {

}

enum MutationResult {
    Dead,
    Alive
}

struct Mutants {
    file: PathBuf,
    line: u32,
    operation: MutationType,
    result: MutationResult
}

pub struct MutationCampaign<'a> {
    contracts_to_mutate: Vec<PathBuf>,
    config: Arc<foundry_config::Config>,
    evm_opts: &'a crate::opts::EvmOpts
}

impl<'a> MutationCampaign<'a> {
    pub fn new(files: Vec<PathBuf>, config: Arc<foundry_config::Config>, evm_opts: &'a crate::opts::EvmOpts) -> MutationCampaign<'a> {
        MutationCampaign {
            contracts_to_mutate: files,
            config,
            evm_opts
        }
    }

    // @todo: return MutationTestOutcome and use it in result.rs / dirty logging for now
    pub fn run(&self) {
        sh_println!("Running mutation tests...").unwrap();

        // Iterate over all contract in contracts_to_mutate
        for contract in &self.contracts_to_mutate {
            // Parse and get the ast (in an Rc for Rayon later)
            self.lex_parse_contract(contract);

            // Rayon from here (enter_parallel)
                // Mutate
                // let mutated_ast = mutate(Rc::clone(&ast));

                // Emit solidity to a new tmp file - let's use tempfile::SpooledTempFile to keep them in memory!
                // let mutated_sol = emit_solidity(mutated_ast);

                // Compile (ideally, we reuse the same artifact folder, and if we're lucky, that's enough to avoid recompiling
                // the whole project on each mutation)

                // Run tests again (see execute_tests, without much of the config)
                
                // Arc(result) update <= write lock I guess

                // Delete tmp file

            println!("{:?}", contract);
        }

        // Print results
    }

    fn lex_parse_contract(&self, target: &PathBuf) {
        let sess = Session::builder().with_silent_emitter(None).build();
        
        let ast = sess.enter(|| -> solar_parse::interface::Result<_> {
            let arena = solar_parse::ast::Arena::new();
            let mut parser = Parser::from_file(&sess, &arena, target)?;
            let ast = parser.parse_file().map_err(|e| e.emit())?;

            let visited = format!("{:?}", ast);

            Ok(visited)
            }).ok();

        dbg!(&ast);
    }
}

pub fn mutate(target: &PathBuf) {
}