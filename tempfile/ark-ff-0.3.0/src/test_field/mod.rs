#[allow(unused)]
pub(crate) use fq::*;
#[allow(unused)]
pub(crate) use fq2::*;
#[allow(unused)]
pub(crate) use fq6::*;
pub(crate) use fr::*;

pub(crate) mod fr {
    /// Copy of BLS12-381's Fr
    use crate::{
        biginteger::BigInteger256 as BigInteger,
        fields::{FftParameters, Fp256, Fp256Parameters, FpParameters},
    };

    #[allow(unused)]
    pub type Fr = Fp256<FrParameters>;

    pub struct FrParameters;

    impl Fp256Parameters for FrParameters {}
    impl FftParameters for FrParameters {
        type BigInt = BigInteger;

        const TWO_ADICITY: u32 = 32;

        #[rustfmt::skip]
        const TWO_ADIC_ROOT_OF_UNITY: BigInteger = BigInteger([
            0xb9b58d8c5f0e466a,
            0x5b1b4c801819d7ec,
            0xaf53ae352a31e64,
            0x5bf3adda19e9b27b,
        ]);
    }
    impl FpParameters for FrParameters {
        /// MODULUS = 52435875175126190479447740508185965837690552500527637822603658699938581184513
        #[rustfmt::skip]
        const MODULUS: BigInteger = BigInteger([
            0xffffffff00000001,
            0x53bda402fffe5bfe,
            0x3339d80809a1d805,
            0x73eda753299d7d48,
        ]);

        const MODULUS_BITS: u32 = 255;

        const CAPACITY: u32 = Self::MODULUS_BITS - 1;

        const REPR_SHAVE_BITS: u32 = 1;

        /// R = 10920338887063814464675503992315976177888879664585288394250266608035967270910
        #[rustfmt::skip]
        const R: BigInteger = BigInteger([
            0x1fffffffe,
            0x5884b7fa00034802,
            0x998c4fefecbc4ff5,
            0x1824b159acc5056f,
        ]);

        #[rustfmt::skip]
        const R2: BigInteger = BigInteger([
            0xc999e990f3f29c6d,
            0x2b6cedcb87925c23,
            0x5d314967254398f,
            0x748d9d99f59ff11,
        ]);

        const INV: u64 = 0xfffffffeffffffff;

        /// GENERATOR = 7
        /// Encoded in Montgomery form, so the value here is
        /// 7 * R % q = 24006497034320510773280787438025867407531605151569380937148207556313189711857
        #[rustfmt::skip]
        const GENERATOR: BigInteger = BigInteger([
            0xefffffff1,
            0x17e363d300189c0f,
            0xff9c57876f8457b0,
            0x351332208fc5a8c4,
        ]);

        #[rustfmt::skip]
        const MODULUS_MINUS_ONE_DIV_TWO: BigInteger = BigInteger([
            0x7fffffff80000000,
            0xa9ded2017fff2dff,
            0x199cec0404d0ec02,
            0x39f6d3a994cebea4,
        ]);

        // T and T_MINUS_ONE_DIV_TWO, where MODULUS - 1 = 2^S * T
        // For T coprime to 2

        // T = (MODULUS - 1) / 2^S =
        // 12208678567578594777604504606729831043093128246378069236549469339647
        #[rustfmt::skip]
        const T: BigInteger = BigInteger([
            0xfffe5bfeffffffff,
            0x9a1d80553bda402,
            0x299d7d483339d808,
            0x73eda753,
        ]);

        // (T - 1) / 2 =
        // 6104339283789297388802252303364915521546564123189034618274734669823
        #[rustfmt::skip]
        const T_MINUS_ONE_DIV_TWO: BigInteger = BigInteger([
            0x7fff2dff7fffffff,
            0x4d0ec02a9ded201,
            0x94cebea4199cec04,
            0x39f6d3a9,
        ]);
    }
}

pub(crate) mod fq {
    /// Copy of BLS12-377's Fq
    use crate::{
        biginteger::BigInteger384 as BigInteger,
        fields::{FftParameters, Fp384, Fp384Parameters, FpParameters},
    };

    pub type Fq = Fp384<FqParameters>;

    pub struct FqParameters;

    impl Fp384Parameters for FqParameters {}
    impl FftParameters for FqParameters {
        type BigInt = BigInteger;

        const TWO_ADICITY: u32 = 46u32;

        #[rustfmt::skip]
        const TWO_ADIC_ROOT_OF_UNITY: BigInteger = BigInteger([
            2022196864061697551u64,
            17419102863309525423u64,
            8564289679875062096u64,
            17152078065055548215u64,
            17966377291017729567u64,
            68610905582439508u64,
        ]);
    }
    impl FpParameters for FqParameters {
        /// MODULUS = 258664426012969094010652733694893533536393512754914660539884262666720468348340822774968888139573360124440321458177
        #[rustfmt::skip]
        const MODULUS: BigInteger = BigInteger([
            0x8508c00000000001,
            0x170b5d4430000000,
            0x1ef3622fba094800,
            0x1a22d9f300f5138f,
            0xc63b05c06ca1493b,
            0x1ae3a4617c510ea,
        ]);

        const MODULUS_BITS: u32 = 377;

        const CAPACITY: u32 = Self::MODULUS_BITS - 1;

        const REPR_SHAVE_BITS: u32 = 7;

        /// R = 85013442423176922659824578519796707547925331718418265885885478904210582549405549618995257669764901891699128663912
        #[rustfmt::skip]
        const R: BigInteger = BigInteger([
            202099033278250856u64,
            5854854902718660529u64,
            11492539364873682930u64,
            8885205928937022213u64,
            5545221690922665192u64,
            39800542322357402u64,
        ]);

        #[rustfmt::skip]
        const R2: BigInteger = BigInteger([
            0xb786686c9400cd22,
            0x329fcaab00431b1,
            0x22a5f11162d6b46d,
            0xbfdf7d03827dc3ac,
            0x837e92f041790bf9,
            0x6dfccb1e914b88,
        ]);

        const INV: u64 = 9586122913090633727u64;

        /// GENERATOR = -5
        /// Encoded in Montgomery form, so the value here is
        /// (-5 * R) % q = 92261639910053574722182574790803529333160366917737991650341130812388023949653897454961487930322210790384999596794
        #[rustfmt::skip]
        const GENERATOR: BigInteger = BigInteger([
            0xfc0b8000000002fa,
            0x97d39cf6e000018b,
            0x2072420fbfa05044,
            0xcbbcbd50d97c3802,
            0xbaf1ec35813f9eb,
            0x9974a2c0945ad2,
        ]);

        #[rustfmt::skip]
        const MODULUS_MINUS_ONE_DIV_TWO: BigInteger = BigInteger([
            0x4284600000000000,
            0xb85aea218000000,
            0x8f79b117dd04a400,
            0x8d116cf9807a89c7,
            0x631d82e03650a49d,
            0xd71d230be28875,
        ]);

        // T and T_MINUS_ONE_DIV_TWO, where MODULUS - 1 = 2^S * T
        // For T coprime to 2

        // T = (MODULUS - 1) // 2^S =
        // 3675842578061421676390135839012792950148785745837396071634149488243117337281387659330802195819009059
        #[rustfmt::skip]
        const T: BigInteger = BigInteger([
            0x7510c00000021423,
            0x88bee82520005c2d,
            0x67cc03d44e3c7bcd,
            0x1701b28524ec688b,
            0xe9185f1443ab18ec,
            0x6b8,
        ]);

        // (T - 1) // 2 =
        // 1837921289030710838195067919506396475074392872918698035817074744121558668640693829665401097909504529
        #[rustfmt::skip]
        const T_MINUS_ONE_DIV_TWO: BigInteger = BigInteger([
            0xba88600000010a11,
            0xc45f741290002e16,
            0xb3e601ea271e3de6,
            0xb80d94292763445,
            0x748c2f8a21d58c76,
            0x35c,
        ]);
    }

    #[allow(dead_code)]
    pub const FQ_ONE: Fq = Fq::new(FqParameters::R);
    #[allow(dead_code)]
    pub const FQ_ZERO: Fq = Fq::new(BigInteger([0, 0, 0, 0, 0, 0]));

    #[test]
    fn test_const_from_repr() {
        use crate::fields::PrimeField;
        let int = BigInteger([
            9586122913090633730,
            4981570305181876224,
            14262076793150106624,
            7033126720376490667,
            699094806891394796,
            0,
        ]);
        let r2 = FqParameters::R2;
        let modulus = FqParameters::MODULUS;
        let inv = FqParameters::INV;

        assert_eq!(
            Fq::from_repr(int).unwrap(),
            Fq::const_from_repr(int, r2, modulus, inv)
        );
    }
}

pub(crate) mod fq2 {
    // Copy of BLS12-377's Fq2
    use super::fq::*;
    use crate::{field_new, fields::*};

    pub type Fq2 = Fp2<Fq2Parameters>;

    pub struct Fq2Parameters;

    impl Fp2Parameters for Fq2Parameters {
        type Fp = Fq;

        /// NONRESIDUE = -5
        #[rustfmt::skip]
        const NONRESIDUE: Fq = field_new!(Fq, "-5");

        /// QUADRATIC_NONRESIDUE = U
        #[rustfmt::skip]
        const QUADRATIC_NONRESIDUE: (Fq, Fq) = (FQ_ZERO, FQ_ONE);

        /// Coefficients for the Frobenius automorphism.
        #[rustfmt::skip]
        const FROBENIUS_COEFF_FP2_C1: &'static [Fq] = &[
            // NONRESIDUE**(((q^0) - 1) / 2)
            FQ_ONE,
            // NONRESIDUE**(((q^1) - 1) / 2)
            field_new!(Fq, "-1"),
        ];

        #[inline(always)]
        fn mul_fp_by_nonresidue(fe: &Self::Fp) -> Self::Fp {
            let original = fe;
            let mut fe = -fe.double();
            fe.double_in_place();
            fe - original
        }
    }

    #[allow(dead_code)]
    pub const FQ2_ZERO: Fq2 = field_new!(Fq2, FQ_ZERO, FQ_ZERO);
    #[allow(dead_code)]
    pub const FQ2_ONE: Fq2 = field_new!(Fq2, FQ_ONE, FQ_ZERO);
}

pub(crate) mod fq6 {
    // Copy of BLS12-377's Fq6
    use super::{fq::*, fq2::*};
    use crate::{field_new, fields::*};

    #[allow(dead_code)]
    pub type Fq6 = Fp6<Fq6Parameters>;

    #[derive(Clone, Copy)]
    pub struct Fq6Parameters;

    impl Fp6Parameters for Fq6Parameters {
        type Fp2Params = Fq2Parameters;

        /// NONRESIDUE = U
        #[rustfmt::skip]
        const NONRESIDUE: Fq2 = field_new!(Fq2, FQ_ZERO, FQ_ONE);

        #[rustfmt::skip]
        const FROBENIUS_COEFF_FP6_C1: &'static [Fq2] = &[
            // Fp2::NONRESIDUE^(((q^0) - 1) / 3)
            field_new!(Fq2, FQ_ONE, FQ_ZERO),
            // Fp2::NONRESIDUE^(((q^1) - 1) / 3)
            field_new!(Fq2,
                field_new!(Fq, "80949648264912719408558363140637477264845294720710499478137287262712535938301461879813459410946"),
                FQ_ZERO,
            ),
            // Fp2::NONRESIDUE^(((q^2) - 1) / 3)
            field_new!(Fq2,
                field_new!(Fq, "80949648264912719408558363140637477264845294720710499478137287262712535938301461879813459410945"),
                FQ_ZERO,
            ),
            // Fp2::NONRESIDUE^(((q^3) - 1) / 3)
            field_new!(Fq2, field_new!(Fq, "-1"), FQ_ZERO),
            // Fp2::NONRESIDUE^(((q^4) - 1) / 3)
            field_new!(Fq2,
                field_new!(Fq, "258664426012969093929703085429980814127835149614277183275038967946009968870203535512256352201271898244626862047231"),
                FQ_ZERO,
            ),
            // Fp2::NONRESIDUE^(((q^5) - 1) / 3)
            field_new!(Fq2,
                field_new!(Fq, "258664426012969093929703085429980814127835149614277183275038967946009968870203535512256352201271898244626862047232"),
                FQ_ZERO,
            ),
        ];
        #[rustfmt::skip]
        const FROBENIUS_COEFF_FP6_C2: &'static [Fq2] = &[
            // Fp2::NONRESIDUE^((2*(q^0) - 2) / 3)
            field_new!(Fq2, FQ_ONE, FQ_ZERO),
            // Fp2::NONRESIDUE^((2*(q^1) - 2) / 3)
            field_new!(Fq2,
                field_new!(Fq, "80949648264912719408558363140637477264845294720710499478137287262712535938301461879813459410945"),
                FQ_ZERO
            ),
            // Fp2::NONRESIDUE^((2*(q^2) - 2) / 3)
            field_new!(Fq2,
                field_new!(Fq, "258664426012969093929703085429980814127835149614277183275038967946009968870203535512256352201271898244626862047231"),
                FQ_ZERO,
            ),
            // Fp2::NONRESIDUE^((2*(q^3) - 2) / 3)
            field_new!(Fq2, FQ_ONE, FQ_ZERO),
            // Fp2::NONRESIDUE^((2*(q^4) - 2) / 3)
            field_new!(Fq2,
                field_new!(Fq, "80949648264912719408558363140637477264845294720710499478137287262712535938301461879813459410945"),
                FQ_ZERO,
            ),
            // Fp2::NONRESIDUE^((2*(q^5) - 2) / 3)
            field_new!(Fq2,
                field_new!(Fq, "258664426012969093929703085429980814127835149614277183275038967946009968870203535512256352201271898244626862047231"),
                FQ_ZERO,
            ),
        ];

        #[inline(always)]
        fn mul_fp2_by_nonresidue(fe: &Fq2) -> Fq2 {
            // Karatsuba multiplication with constant other = u.
            let c0 = Fq2Parameters::mul_fp_by_nonresidue(&fe.c1);
            let c1 = fe.c0;
            field_new!(Fq2, c0, c1)
        }
    }
}
