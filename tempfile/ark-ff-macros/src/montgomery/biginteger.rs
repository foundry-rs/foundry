use quote::quote;

pub(super) fn add_with_carry_impl(num_limbs: usize) -> proc_macro2::TokenStream {
    let mut body = proc_macro2::TokenStream::new();
    body.extend(quote! {
        use ark_ff::biginteger::arithmetic::adc_for_add_with_carry as adc;
        let mut carry = 0;
    });
    for i in 0..num_limbs {
        body.extend(quote! {
            carry = adc(&mut a.0[#i], b.0[#i], carry);
        });
    }
    body.extend(quote! {
        carry != 0
    });
    quote! {
        #[inline(always)]
        fn __add_with_carry(
            a: &mut B,
            b: & B,
        ) -> bool {
            #body
        }
    }
}

pub(super) fn sub_with_borrow_impl(num_limbs: usize) -> proc_macro2::TokenStream {
    let mut body = proc_macro2::TokenStream::new();
    body.extend(quote! {
        use ark_ff::biginteger::arithmetic::sbb_for_sub_with_borrow as sbb;
        let mut borrow = 0;
    });
    for i in 0..num_limbs {
        body.extend(quote! {
            borrow = sbb(&mut a.0[#i], b.0[#i], borrow);
        });
    }
    body.extend(quote! {
        borrow != 0
    });
    quote! {
        #[inline(always)]
        fn __sub_with_borrow(
            a: &mut B,
            b: & B,
        ) -> bool {
            #body
        }
    }
}

pub(super) fn subtract_modulus_impl(
    modulus: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    quote! {
        #[inline(always)]
        fn __subtract_modulus(a: &mut F) {
            if a.is_geq_modulus() {
                __sub_with_borrow(&mut a.0, &#modulus);
            }
        }

        #[inline(always)]
        fn __subtract_modulus_with_carry(a: &mut F, carry: bool) {
            if a.is_geq_modulus() || carry {
                __sub_with_borrow(&mut a.0, &#modulus);
            }
        }
    }
}
