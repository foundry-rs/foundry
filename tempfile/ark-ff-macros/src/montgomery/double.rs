pub(super) fn double_in_place_impl(modulus_has_spare_bit: bool) -> proc_macro2::TokenStream {
    if modulus_has_spare_bit {
        quote::quote! {
            // This cannot exceed the backing capacity.
            a.0.mul2();
            // However, it may need to be reduced.
            __subtract_modulus(a);
        }
    } else {
        quote::quote! {
            // This cannot exceed the backing capacity.
            let c = a.0.mul2();
            // However, it may need to be reduced.
            __subtract_modulus_with_carry(a, c);
        }
    }
}
