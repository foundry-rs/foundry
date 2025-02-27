// Generate mutants then run tests (reuse the whole unit test flow for now, including compilation to select mutants)
// Use Solar: 
use solar_parse::{
    token::{Token, TokenKind},
    Lexer,
    ast::{
        interface::{self, Session},
        Arena, CommentKind, Item, ItemKind,
    },
    Parser,
};

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

pub struct MutationCampaign {
    contracts_to_mutate: Vec<PathBuf>, 
}

impl MutationCampaign {
    pub fn new(files: Vec<PathBuf>) -> MutationCampaign {
        MutationCampaign { contracts_to_mutate: files }
    }
}

pub fn mutate(target: &PathBuf) {
    let lexer = Lexer::new(
        &Session::builder().with_silent_emitter(None).build(),
        target.to_str().unwrap(),
    );
}