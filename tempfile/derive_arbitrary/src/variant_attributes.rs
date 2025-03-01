use crate::ARBITRARY_ATTRIBUTE_NAME;
use syn::*;

pub fn not_skipped(variant: &&Variant) -> bool {
    !should_skip(variant)
}

fn should_skip(Variant { attrs, .. }: &Variant) -> bool {
    attrs
        .iter()
        .filter_map(|attr| {
            attr.path()
                .is_ident(ARBITRARY_ATTRIBUTE_NAME)
                .then(|| attr.parse_args::<Meta>())
                .and_then(Result::ok)
        })
        .any(|meta| match meta {
            Meta::Path(path) => path.is_ident("skip"),
            _ => false,
        })
}
