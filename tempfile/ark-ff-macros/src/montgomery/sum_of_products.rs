use quote::quote;

pub(super) fn sum_of_products_impl(num_limbs: usize, modulus: &[u64]) -> proc_macro2::TokenStream {
    let modulus_size =
        (((num_limbs - 1) * 64) as u32 + (64 - modulus[num_limbs - 1].leading_zeros())) as usize;
    let mut body = proc_macro2::TokenStream::new();
    // Adapted from https://github.com/zkcrypto/bls12_381/pull/84 by @str4d.

    // For a single `a x b` multiplication, operand scanning (schoolbook) takes each
    // limb of `a` in turn, and multiplies it by all of the limbs of `b` to compute
    // the result as a double-width intermediate representation, which is then fully
    // reduced at the carry. Here however we have pairs of multiplications (a_i, b_i),
    // the results of which are summed.
    //
    // The intuition for this algorithm is two-fold:
    // - We can interleave the operand scanning for each pair, by processing the jth
    //   limb of each `a_i` together. As these have the same offset within the overall
    //   operand scanning flow, their results can be summed directly.
    // - We can interleave the multiplication and reduction steps, resulting in a
    //   single bitshift by the limb size after each iteration. This means we only
    //   need to store a single extra limb overall, instead of keeping around all the
    //   intermediate results and eventually having twice as many limbs.

    if modulus_size >= 64 * num_limbs - 1 {
        quote! {
            a.iter().zip(b).map(|(a, b)| *a * b).sum()
        }
    } else {
        let mut inner_loop_body = proc_macro2::TokenStream::new();
        for k in 1..num_limbs {
            inner_loop_body.extend(quote! {
                result.0[#k] = fa::mac_with_carry(result.0[#k], a.0[j], b.0[#k], &mut carry2);
            });
        }
        let mut mont_red_body = proc_macro2::TokenStream::new();
        for (i, modulus_i) in modulus.iter().enumerate().take(num_limbs).skip(1) {
            mont_red_body.extend(quote! {
                result.0[#i - 1] = fa::mac_with_carry(result.0[#i], k, #modulus_i, &mut carry2);
            });
        }
        let modulus_0 = modulus[0];
        let chunk_size = 2 * (num_limbs * 64 - modulus_size) - 1;
        body.extend(quote! {
            if M <= #chunk_size {
                // Algorithm 2, line 2
                let result = (0..#num_limbs).fold(BigInt::zero(), |mut result, j| {
                    // Algorithm 2, line 3
                    let mut carry_a = 0;
                    let mut carry_b = 0;
                    for (a, b) in a.iter().zip(b) {
                        let a = &a.0;
                        let b = &b.0;
                        let mut carry2 = 0;
                        result.0[0] = fa::mac(result.0[0], a.0[j], b.0[0], &mut carry2);
                        #inner_loop_body
                        carry_b = fa::adc(&mut carry_a, carry_b, carry2);
                    }

                    let k = result.0[0].wrapping_mul(Self::INV);
                    let mut carry2 = 0;
                    fa::mac_discard(result.0[0], k, #modulus_0, &mut carry2);
                    #mont_red_body
                    result.0[#num_limbs - 1] = fa::adc_no_carry(carry_a, carry_b, &mut carry2);
                    result
                });
                let mut result = F::new_unchecked(result);
                __subtract_modulus(&mut result);
                debug_assert_eq!(
                    a.iter().zip(b).map(|(a, b)| *a * b).sum::<F>(),
                    result
                );
                result
            } else {
                a.chunks(#chunk_size).zip(b.chunks(#chunk_size)).map(|(a, b)| {
                    if a.len() == #chunk_size {
                        Self::sum_of_products::<#chunk_size>(a.try_into().unwrap(), b.try_into().unwrap())
                    } else {
                        a.iter().zip(b).map(|(a, b)| *a * b).sum()
                    }
                }).sum()
            }


        });
        body
    }
}
