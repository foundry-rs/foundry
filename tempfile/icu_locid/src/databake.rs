// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use crate::LanguageIdentifier;
use alloc::string::ToString;
use databake::*;

impl Bake for LanguageIdentifier {
    fn bake(&self, env: &CrateEnv) -> TokenStream {
        env.insert("icu_locid");
        let repr = self.to_string();
        if self.variants.len() <= 1 {
            quote! {
                icu_locid::langid!(#repr)
            }
        } else {
            quote! {
                icu_locid::LanguageIdentifier::from_str(#repr).unwrap()
            }
        }
    }
}
