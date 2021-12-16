use colored::*;
use rustyline::{
    completion::Completer, highlight::Highlighter, hint::Hinter, CompletionType, EditMode, Editor,
};
use std::borrow::Cow;

/// Returns a new Vi edit style editor
pub fn editor() -> Editor<SolReplCompleter> {
    let config = rustyline::Config::builder()
        .edit_mode(EditMode::Vi)
        .completion_type(CompletionType::List)
        .keyseq_timeout(0) // https://github.com/kkawakam/rustyline/issues/371
        .build();
    Editor::with_config(config)
}
/// Type responsible to provide hints, complete, highlight
#[derive(Default)]
pub struct SolReplCompleter {
    /// Stores the available command context
    context: Vec<String>,
}

impl SolReplCompleter {
    pub fn set(&mut self, context: Vec<String>) {
        self.context = context;
    }

    /// Creates a new editor and register the sol repl as helper
    pub fn into_editor(self) -> Editor<SolReplCompleter> {
        let mut editor = editor();
        editor.set_helper(Some(self));
        editor
    }
}

impl rustyline::validate::Validator for SolReplCompleter {}

impl Completer for SolReplCompleter {
    type Candidate = String;
}

impl Highlighter for SolReplCompleter {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> Cow<'b, str> {
        prompt.yellow().to_string().into()
    }
}

impl Hinter for SolReplCompleter {
    type Hint = String;
}

impl rustyline::Helper for SolReplCompleter {}
