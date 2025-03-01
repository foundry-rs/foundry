use std::collections::HashSet;

use proc_macro2::Span as Span2;
use syn::{
    visit::{visit_item_trait, Visit},
    Block, Ident, ItemTrait, Lifetime,
};

/// The type parameter used in the proxy type. Usually, one would just use `T`,
/// but this could conflict with type parameters on the trait.
///
/// Why do we have to care about this? Why about hygiene? In the first version
/// of stable proc_macros, only call site spans are included. That means that
/// we cannot generate spans that do not conflict with any other ident the user
/// wrote. Once proper hygiene is available to proc_macros, this should be
/// changed.
const PROXY_TY_PARAM_NAME: &str = "__AutoImplProxyT";

/// The lifetime parameter used in the proxy type if the proxy type is `&` or
/// `&mut`. For more information see `PROXY_TY_PARAM_NAME`.
const PROXY_LT_PARAM_NAME: &str = "'__auto_impl_proxy_lifetime";

/// We need to introduce our own type and lifetime parameter. Regardless of
/// what kind of hygiene we use for the parameter, it would be nice (for docs
/// and compiler errors) if the names are as simple as possible ('a and T, for
/// example).
///
/// This function searches for names that we can use. Such a name must not
/// conflict with any other name we'll use in the `impl` block. Luckily, we
/// know all those names in advance.
///
/// The idea is to collect all names that might conflict with our names, store
/// them in a set and later check which name we can use. If we can't use a simple
/// name, we'll use the ugly `PROXY_TY_PARAM_NAME` and `PROXY_LT_PARAM_NAME`.
///
/// This method returns two idents: (type_parameter, lifetime_parameter).
pub(crate) fn find_suitable_param_names(trait_def: &ItemTrait) -> (Ident, Lifetime) {
    // Define the visitor that just collects names
    struct IdentCollector<'ast> {
        ty_names: HashSet<&'ast Ident>,
        lt_names: HashSet<&'ast Ident>,
    }

    impl<'ast> Visit<'ast> for IdentCollector<'ast> {
        fn visit_ident(&mut self, i: &'ast Ident) {
            self.ty_names.insert(i);
        }

        // We overwrite this to make sure to put lifetime names into
        // `lt_names`. We also don't recurse, so `visit_ident` won't be called
        // for lifetime names.
        fn visit_lifetime(&mut self, lt: &'ast Lifetime) {
            self.lt_names.insert(&lt.ident);
        }

        // Visiting a block just does nothing. It is the default body of a method
        // in the trait. But since that block won't be in the impl block, we can
        // just ignore it.
        fn visit_block(&mut self, _: &'ast Block) {}
    }

    // Create the visitor and visit the trait
    let mut visitor = IdentCollector {
        ty_names: HashSet::new(),
        lt_names: HashSet::new(),
    };
    visit_item_trait(&mut visitor, trait_def);

    fn char_to_ident(c: u8) -> Ident {
        let arr = [c];
        let s = ::std::str::from_utf8(&arr).unwrap();
        Ident::new(s, param_span())
    }

    // Find suitable type name (T..=Z and A..=S)
    let ty_name = (b'T'..=b'Z')
        .chain(b'A'..=b'S')
        .map(char_to_ident)
        .find(|i| !visitor.ty_names.contains(i))
        .unwrap_or_else(|| Ident::new(PROXY_TY_PARAM_NAME, param_span()));

    // Find suitable lifetime name ('a..='z)
    let lt_name = (b'a'..=b'z')
        .map(char_to_ident)
        .find(|i| !visitor.lt_names.contains(i))
        .unwrap_or_else(|| Ident::new(PROXY_LT_PARAM_NAME, param_span()));
    let lt = Lifetime {
        apostrophe: param_span(),
        ident: lt_name,
    };

    (ty_name, lt)
}

fn param_span() -> Span2 {
    Span2::call_site()
}
