// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

// An example application which uses icu_uniset to test what blocks of
// Basic Multilingual Plane a character belongs to.
//
// In this example we use `CodePointInversionListBuilder` to construct just the first
// two blocks of the first plane, and use an instance of a `BMPBlockSelector`
// to retrieve which of those blocks each character of a string belongs to.
//
// This is a simple example of the API use and is severely oversimplified
// compared to real Unicode block selection.

#![no_main] // https://github.com/unicode-org/icu4x/issues/395

icu_benchmark_macros::static_setup!();

use icu_collections::codepointinvlist::{CodePointInversionList, CodePointInversionListBuilder};

fn get_basic_latin_block() -> CodePointInversionList<'static> {
    let mut builder = CodePointInversionListBuilder::new();
    builder.add_range(&('\u{0000}'..='\u{007F}'));
    builder.build()
}

fn get_latin1_supplement_block() -> CodePointInversionList<'static> {
    let mut builder = CodePointInversionListBuilder::new();
    builder.add_range(&('\u{0080}'..='\u{00FF}'));
    builder.build()
}

#[derive(Copy, Clone, Debug)]
enum BmpBlock {
    Basic,
    Latin1Supplement,
    Unknown,
}

struct BmpBlockSelector<'data> {
    blocks: Vec<(BmpBlock, CodePointInversionList<'data>)>,
}

impl<'data> BmpBlockSelector<'data> {
    pub fn new() -> Self {
        let blocks = vec![
            (BmpBlock::Basic, get_basic_latin_block()),
            (BmpBlock::Latin1Supplement, get_latin1_supplement_block()),
        ];
        BmpBlockSelector { blocks }
    }

    pub fn select(&self, input: char) -> BmpBlock {
        for (block, set) in &self.blocks {
            if set.contains(input) {
                return *block;
            }
        }
        BmpBlock::Unknown
    }
}

fn print(_input: &str) {
    #[cfg(debug_assertions)]
    println!("{_input}");
}

#[no_mangle]
fn main(_argc: isize, _argv: *const *const u8) -> isize {
    icu_benchmark_macros::main_setup!();
    let selector = BmpBlockSelector::new();

    let sample = "Welcome to MyName©®, Алексей!";

    let mut result = vec![];

    for ch in sample.chars() {
        result.push((ch, selector.select(ch)));
    }

    print("\n====== Unicode BMP Block Selector example ============");
    for (ch, block) in result {
        print(&format!("{ch}: {block:#?}"));
    }

    0
}
