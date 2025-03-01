use bon::{bon, builder, Builder};

struct NoTraitImpls;

#[derive(Builder)]
#[builder(derive(Clone, Debug))]
struct StructContainsNonTrait {
    #[builder(start_fn)]
    no_impl_start_fn: NoTraitImpls,

    no_impls_required: NoTraitImpls,

    no_impl_optional: Option<NoTraitImpls>,

    #[builder(default = NoTraitImpls)]
    no_impl_optional_2: NoTraitImpls,

    x: u32,
}

#[builder(derive(Clone, Debug))]
fn fn_contains_non_trait(
    #[builder(start_fn)] //
    _no_impl_start_fn: NoTraitImpls,

    _no_impls_required: NoTraitImpls,

    _no_impl_optional: Option<NoTraitImpls>,

    #[builder(default = NoTraitImpls)] //
    _no_impl_optional_2: NoTraitImpls,

    _x: u32,
) {
}

#[bon]
impl StructContainsNonTrait {
    #[builder(derive(Clone, Debug))]
    fn method_contains_non_trait(
        self,

        #[builder(start_fn)] _no_impl_start_fn: NoTraitImpls,

        _no_impls_required: NoTraitImpls,

        _no_impl_optional: Option<NoTraitImpls>,

        #[builder(default = NoTraitImpls)] //
        _no_impl_optional_2: NoTraitImpls,

        _x: u32,
    ) {
    }
}

#[derive(Builder)]
#[builder(derive())]
struct EmptyDerives {}

#[derive(Builder)]
#[builder(derive(Clone()))]
struct EmptyParamsForDerive {}

#[derive(Builder)]
#[builder(derive(Clone(bounds {})))]
struct WrongDelimInBounds {}

fn main() {}
