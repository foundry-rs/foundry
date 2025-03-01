pub(super) fn add_assign_impl(modulus_has_spare_bit: bool) -> proc_macro2::TokenStream {
    if modulus_has_spare_bit {
        quote::quote! {
            __add_with_carry(&mut a.0, &b.0);
            __subtract_modulus(a);
        }
    } else {
        quote::quote! {
            let c = __add_with_carry(&mut a.0, &b.0);
            __subtract_modulus_with_carry(a, c);
        }
    }
}
