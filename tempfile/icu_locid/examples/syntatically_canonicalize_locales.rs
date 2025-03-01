// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

// A sample application which takes a comma separated list of locales,
// makes them syntatically canonical and serializes the list back into a comma separated list.

icu_benchmark_macros::static_setup!();

use std::env;

use icu_locid::Locale;

const DEFAULT_INPUT: &str = "sr-cyrL-rS, es-mx, und-arab-u-ca-Buddhist";

fn syntatically_canonicalize_locales(input: &str) -> String {
    // Split input string and canonicalize each locale identifier.
    let canonical_locales: Vec<String> = input
        .split(',')
        .filter_map(|s| Locale::canonicalize(s.trim()).ok())
        .collect();

    canonical_locales.join(", ")
}

fn main() {
    icu_benchmark_macros::main_setup!();
    let args: Vec<String> = env::args().collect();

    let input = if let Some(input) = args.get(1) {
        input.as_str()
    } else {
        DEFAULT_INPUT
    };
    let _output = syntatically_canonicalize_locales(input);

    #[cfg(debug_assertions)]
    println!("\nInput: {input}\nOutput: {_output}");
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_OUTPUT: &str = "sr-Cyrl-RS, es-MX, und-Arab-u-ca-buddhist";

    #[test]
    fn ensure_default_output() {
        assert_eq!(
            syntatically_canonicalize_locales(DEFAULT_INPUT),
            DEFAULT_OUTPUT
        );
    }
}
