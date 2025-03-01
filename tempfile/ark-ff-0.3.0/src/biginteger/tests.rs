use crate::{biginteger::BigInteger, UniformRand};
use num_bigint::BigUint;

fn biginteger_arithmetic_test<B: BigInteger>(a: B, b: B, zero: B) {
    // zero == zero
    assert_eq!(zero, zero);

    // zero.is_zero() == true
    assert_eq!(zero.is_zero(), true);

    // a == a
    assert_eq!(a, a);

    // a + 0 = a
    let mut a0_add = a.clone();
    a0_add.add_nocarry(&zero);
    assert_eq!(a0_add, a);

    // a - 0 = a
    let mut a0_sub = a.clone();
    a0_sub.sub_noborrow(&zero);
    assert_eq!(a0_sub, a);

    // a - a = 0
    let mut aa_sub = a.clone();
    aa_sub.sub_noborrow(&a);
    assert_eq!(aa_sub, zero);

    // a + b = b + a
    let mut ab_add = a.clone();
    ab_add.add_nocarry(&b);
    let mut ba_add = b.clone();
    ba_add.add_nocarry(&a);
    assert_eq!(ab_add, ba_add);
}

fn biginteger_bits_test<B: BigInteger>() {
    let mut one = B::from(1u64);
    assert!(one.get_bit(0));
    assert!(!one.get_bit(1));
    one.muln(5);
    let thirty_two = one;
    assert!(!thirty_two.get_bit(0));
    assert!(!thirty_two.get_bit(1));
    assert!(!thirty_two.get_bit(2));
    assert!(!thirty_two.get_bit(3));
    assert!(!thirty_two.get_bit(4));
    assert!(thirty_two.get_bit(5), "{:?}", thirty_two);
}

fn biginteger_bytes_test<B: BigInteger>() {
    let mut bytes = [0u8; 256];
    let mut rng = ark_std::test_rng();
    let x: B = UniformRand::rand(&mut rng);
    x.write(bytes.as_mut()).unwrap();
    let y = B::read(bytes.as_ref()).unwrap();
    assert_eq!(x, y);
}

fn biginteger_conversion_test<B: BigInteger>() {
    let mut rng = ark_std::test_rng();

    let x: B = UniformRand::rand(&mut rng);
    let x_bigint: BigUint = x.clone().into();
    let x_recovered = B::try_from(x_bigint).ok().unwrap();

    assert_eq!(x, x_recovered);
}

fn test_biginteger<B: BigInteger>(zero: B) {
    let mut rng = ark_std::test_rng();
    let a: B = UniformRand::rand(&mut rng);
    let b: B = UniformRand::rand(&mut rng);
    biginteger_arithmetic_test(a, b, zero);
    biginteger_bytes_test::<B>();
    biginteger_bits_test::<B>();
    biginteger_conversion_test::<B>();
}

#[test]
fn test_biginteger64() {
    use crate::biginteger::BigInteger64 as B;
    test_biginteger(B::new([0u64; 1]));
}

#[test]
fn test_biginteger128() {
    use crate::biginteger::BigInteger128 as B;
    test_biginteger(B::new([0u64; 2]));
}

#[test]
fn test_biginteger256() {
    use crate::biginteger::BigInteger256 as B;
    test_biginteger(B::new([0u64; 4]));
}

#[test]
fn test_biginteger384() {
    use crate::biginteger::BigInteger384 as B;
    test_biginteger(B::new([0u64; 6]));
}

#[test]
fn test_biginteger448() {
    use crate::biginteger::BigInteger448 as B;
    test_biginteger(B::new([0u64; 7]));
}

#[test]
fn test_biginteger768() {
    use crate::biginteger::BigInteger768 as B;
    test_biginteger(B::new([0u64; 12]));
}

#[test]
fn test_biginteger832() {
    use crate::biginteger::BigInteger832 as B;
    test_biginteger(B::new([0u64; 13]));
}
