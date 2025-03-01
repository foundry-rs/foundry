use impl_trait_for_tuples::impl_for_tuples;

#[test]
fn is_implemented_for_tuples() {
    #[impl_for_tuples(5)]
    trait EmptyTrait {}

    struct EmptyTraitImpl;

    impl EmptyTrait for EmptyTraitImpl {}

    fn test<T: EmptyTrait>() {}

    test::<()>();
    test::<EmptyTraitImpl>();
    test::<(EmptyTraitImpl, EmptyTraitImpl, EmptyTraitImpl)>();
    test::<(
        EmptyTraitImpl,
        EmptyTraitImpl,
        EmptyTraitImpl,
        EmptyTraitImpl,
        EmptyTraitImpl,
    )>();
    test::<(
        (
            EmptyTraitImpl,
            EmptyTraitImpl,
            EmptyTraitImpl,
            EmptyTraitImpl,
            EmptyTraitImpl,
        ),
        (
            EmptyTraitImpl,
            EmptyTraitImpl,
            EmptyTraitImpl,
            EmptyTraitImpl,
            EmptyTraitImpl,
        ),
    )>();
}

#[test]
fn full_automatic_accept_empty_tuple_return() {
    #[impl_for_tuples(5)]
    trait EmptyTupleReturn {
        fn test() -> ();
    }
}

#[test]
fn full_automatic_unsafe_trait() {
    #[impl_for_tuples(5)]
    unsafe trait UnsafeTrait {
        fn test();
    }
}

#[test]
fn accept_attributes() {
    /// Hey, I'm an attribute!
    #[cfg(test)]
    #[impl_for_tuples(5)]
    trait Attributes {}

    trait Test {}

    /// Hey, I'm an attribute!
    #[cfg(test)]
    #[impl_for_tuples(5)]
    impl Test for Tuple {}
}

#[test]
fn is_implemented_for_tuples_with_semi() {
    trait EmptyTrait {}

    #[impl_for_tuples(5)]
    impl EmptyTrait for Tuple {}

    struct EmptyTraitImpl;

    impl EmptyTrait for EmptyTraitImpl {}

    fn test<T: EmptyTrait>() {}

    test::<()>();
    test::<EmptyTraitImpl>();
    test::<(EmptyTraitImpl, EmptyTraitImpl, EmptyTraitImpl)>();
    test::<(
        EmptyTraitImpl,
        EmptyTraitImpl,
        EmptyTraitImpl,
        EmptyTraitImpl,
        EmptyTraitImpl,
    )>();
    test::<(
        (
            EmptyTraitImpl,
            EmptyTraitImpl,
            EmptyTraitImpl,
            EmptyTraitImpl,
            EmptyTraitImpl,
        ),
        (
            EmptyTraitImpl,
            EmptyTraitImpl,
            EmptyTraitImpl,
            EmptyTraitImpl,
            EmptyTraitImpl,
        ),
    )>();
}

#[test]
fn trait_with_static_functions() {
    #[impl_for_tuples(50)]
    trait TraitWithFunctions {
        fn function(counter: &mut u32);
        fn function_with_args(data: String, l: u32);
        fn function_with_args_wild(_: String, _: u32);
    }

    struct Impl;

    impl TraitWithFunctions for Impl {
        fn function(counter: &mut u32) {
            *counter += 1;
        }
        fn function_with_args(_: String, _: u32) {}
        fn function_with_args_wild(_: String, _: u32) {}
    }

    fn test<T: TraitWithFunctions>(counter: &mut u32) {
        T::function(counter);
    }

    let mut counter = 0;
    test::<(Impl, Impl, Impl)>(&mut counter);
    assert_eq!(3, counter);

    let mut counter = 0;
    test::<(Impl, Impl, Impl, Impl, Impl)>(&mut counter);
    assert_eq!(5, counter);
}

#[test]
fn trait_with_functions() {
    #[impl_for_tuples(50)]
    trait TraitWithFunctions {
        fn function(&self, counter: &mut u32);
        fn function_with_args(&self, data: String, l: u32);
        fn function_with_args_wild(self, _: String, _: u32);
    }

    struct Impl;

    impl TraitWithFunctions for Impl {
        fn function(&self, counter: &mut u32) {
            *counter += 1;
        }
        fn function_with_args(&self, _: String, _: u32) {}
        fn function_with_args_wild(self, _: String, _: u32) {}
    }

    fn test<T: TraitWithFunctions>(data: T, counter: &mut u32) {
        data.function(counter);
    }

    let mut counter = 0;
    test((Impl, Impl, Impl), &mut counter);
    assert_eq!(3, counter);

    let mut counter = 0;
    test((Impl, Impl, Impl, Impl, Impl), &mut counter);
    assert_eq!(5, counter);
}

#[test]
fn trait_with_static_functions_and_generics() {
    #[impl_for_tuples(50)]
    trait TraitWithFunctions<T, N> {
        fn function(counter: &mut u32);
        fn function_with_args(data: String, l: T);
        fn function_with_args_wild(_: String, _: &N);
    }

    struct Impl;

    impl<T, N> TraitWithFunctions<T, N> for Impl {
        fn function(counter: &mut u32) {
            *counter += 1;
        }
        fn function_with_args(_: String, _: T) {}
        fn function_with_args_wild(_: String, _: &N) {}
    }

    fn test<T: TraitWithFunctions<u32, Impl>>(counter: &mut u32) {
        T::function(counter);
    }

    let mut counter = 0;
    test::<()>(&mut counter);
    assert_eq!(0, counter);

    let mut counter = 0;
    test::<Impl>(&mut counter);
    assert_eq!(1, counter);

    let mut counter = 0;
    test::<(Impl, Impl, Impl)>(&mut counter);
    assert_eq!(3, counter);

    let mut counter = 0;
    test::<(Impl, Impl, Impl, Impl, Impl)>(&mut counter);
    assert_eq!(5, counter);
}

#[test]
fn trait_with_return_type() {
    trait TraitWithReturnType {
        fn function(counter: &mut u32) -> Result<(), ()>;
    }

    #[impl_for_tuples(50)]
    impl TraitWithReturnType for Tuple {
        fn function(counter: &mut u32) -> Result<(), ()> {
            for_tuples!( #( Tuple::function(counter)?; )* );
            Ok(())
        }
    }

    struct Impl;

    impl TraitWithReturnType for Impl {
        fn function(counter: &mut u32) -> Result<(), ()> {
            *counter += 1;
            Ok(())
        }
    }

    fn test<T: TraitWithReturnType>(counter: &mut u32) {
        T::function(counter).unwrap();
    }

    let mut counter = 0;
    test::<()>(&mut counter);
    assert_eq!(0, counter);

    let mut counter = 0;
    test::<Impl>(&mut counter);
    assert_eq!(1, counter);

    let mut counter = 0;
    test::<(Impl, Impl, Impl)>(&mut counter);
    assert_eq!(3, counter);

    let mut counter = 0;
    test::<(Impl, Impl, Impl, Impl, Impl)>(&mut counter);
    assert_eq!(5, counter);
}

#[test]
fn trait_with_associated_type() {
    trait TraitWithAssociatedType {
        type Ret;
        fn function(counter: &mut u32) -> Self::Ret;
    }

    #[impl_for_tuples(50)]
    impl TraitWithAssociatedType for Tuple {
        for_tuples!( type Ret = ( #( Tuple::Ret ),* ); );
        fn function(counter: &mut u32) -> Self::Ret {
            for_tuples!( ( #( Tuple::function(counter) ),* ) )
        }
    }

    struct Impl;

    impl TraitWithAssociatedType for Impl {
        type Ret = u32;
        fn function(counter: &mut u32) -> u32 {
            *counter += 1;
            *counter
        }
    }

    fn test<T: TraitWithAssociatedType>(counter: &mut u32) -> T::Ret {
        T::function(counter)
    }

    let mut counter = 0;
    let res = test::<()>(&mut counter);
    assert_eq!(0, counter);
    assert_eq!((), res);

    let mut counter = 0;
    let res = test::<Impl>(&mut counter);
    assert_eq!(1, counter);
    assert_eq!((1), res);

    let mut counter = 0;
    let res = test::<(Impl, Impl, Impl)>(&mut counter);
    assert_eq!(3, counter);
    assert_eq!((1, 2, 3), res);

    let mut counter = 0;
    let res = test::<(Impl, Impl, Impl, Impl, Impl)>(&mut counter);
    assert_eq!(5, counter);
    assert_eq!((1, 2, 3, 4, 5), res);
}

#[test]
fn trait_with_associated_type_and_generics() {
    trait TraitWithAssociatedType<T, R> {
        type Ret;
        fn function(counter: &mut u32) -> Self::Ret;
    }

    #[impl_for_tuples(50)]
    impl<T, R> TraitWithAssociatedType<T, R> for Tuple {
        for_tuples!( type Ret = ( #( Tuple::Ret ),* ); );
        fn function(counter: &mut u32) -> Self::Ret {
            for_tuples!( ( #( Tuple::function(counter) ),* ) )
        }
    }

    struct Impl;

    impl TraitWithAssociatedType<u32, Self> for Impl {
        type Ret = u32;
        fn function(counter: &mut u32) -> u32 {
            *counter += 1;
            *counter
        }
    }

    fn test<T: TraitWithAssociatedType<u32, Impl>>(counter: &mut u32) -> T::Ret {
        T::function(counter)
    }

    let mut counter = 0;
    let res = test::<()>(&mut counter);
    assert_eq!(0, counter);
    assert_eq!((), res);

    let mut counter = 0;
    let res = test::<Impl>(&mut counter);
    assert_eq!(1, counter);
    assert_eq!((1), res);

    let mut counter = 0;
    let res = test::<(Impl, Impl, Impl)>(&mut counter);
    assert_eq!(3, counter);
    assert_eq!((1, 2, 3), res);

    let mut counter = 0;
    let res = test::<(Impl, Impl, Impl, Impl, Impl)>(&mut counter);
    assert_eq!(5, counter);
    assert_eq!((1, 2, 3, 4, 5), res);
}

#[test]
fn trait_with_associated_type_and_self() {
    trait TraitWithAssociatedTypeAndSelf {
        type Ret;
        fn function(&self, counter: &mut u32) -> Self::Ret;
    }

    #[impl_for_tuples(50)]
    impl TraitWithAssociatedTypeAndSelf for Tuple {
        for_tuples!( type Ret = ( #( Tuple::Ret ),* ); );
        fn function(&self, counter: &mut u32) -> Self::Ret {
            for_tuples!( ( #( Tuple.function(counter) ),* ) )
        }
    }

    struct Impl;

    impl TraitWithAssociatedTypeAndSelf for Impl {
        type Ret = u32;
        fn function(&self, counter: &mut u32) -> u32 {
            *counter += 1;
            *counter
        }
    }

    fn test<T: TraitWithAssociatedTypeAndSelf>(data: T, counter: &mut u32) -> T::Ret {
        data.function(counter)
    }

    let mut counter = 0;
    let res = test((), &mut counter);
    assert_eq!(0, counter);
    assert_eq!((), res);

    let mut counter = 0;
    let res = test(Impl, &mut counter);
    assert_eq!(1, counter);
    assert_eq!((1), res);

    let mut counter = 0;
    let res = test((Impl, Impl, Impl), &mut counter);
    assert_eq!(3, counter);
    assert_eq!((1, 2, 3), res);

    let mut counter = 0;
    let res = test((Impl, Impl, Impl, Impl, Impl), &mut counter);
    assert_eq!(5, counter);
    assert_eq!((1, 2, 3, 4, 5), res);
}

#[test]
fn semi_automatic_tuple_access() {
    trait Trait {
        type Arg;

        fn test(arg: Self::Arg);
    }

    #[impl_for_tuples(5)]
    impl Trait for Tuple {
        for_tuples!( type Arg = ( #( Tuple::Arg ),* ); );
        fn test(arg: Self::Arg) {
            for_tuples!( #( Tuple::test(arg.Tuple); )* )
        }
    }
}

#[test]
fn semi_automatic_with_custom_where_clause() {
    trait Trait {
        type Arg;

        fn test(arg: Self::Arg);
    }

    #[impl_for_tuples(5)]
    impl Trait for Tuple {
        for_tuples!( where #( Tuple: Trait<Arg=u32> ),* );
        type Arg = u32;

        fn test(arg: Self::Arg) {
            for_tuples!( #( Tuple::test(arg); )* )
        }
    }
}

#[test]
fn for_tuples_in_nested_expr_works() {
    trait Trait {
        type Arg;
        type Res;

        fn test(arg: Self::Arg) -> Result<Self::Res, ()>;
    }

    #[impl_for_tuples(5)]
    impl Trait for Tuple {
        for_tuples!( where #( Tuple: Trait<Arg=u32> ),* );
        type Arg = u32;
        for_tuples!( type Res = ( #( Tuple::Res ),* ); );

        fn test(arg: Self::Arg) -> Result<Self::Res, ()> {
            Ok(for_tuples!( ( #( Tuple::test(arg)? ),* ) ))
        }
    }
}

#[test]
fn test_separators() {
    trait Trait {
        fn plus() -> u32;
        fn minus() -> i32;
        fn star() -> u32;
    }

    #[impl_for_tuples(1, 5)]
    impl Trait for Tuple {
        fn plus() -> u32 {
            for_tuples!( #( Tuple::plus() )+* )
        }

        fn minus() -> i32 {
            for_tuples!( #( Tuple::minus() )-* )
        }

        fn star() -> u32 {
            for_tuples!( #( Tuple::star() )** )
        }
    }

    struct Impl;

    impl Trait for Impl {
        fn plus() -> u32 {
            5
        }

        fn minus() -> i32 {
            1000
        }

        fn star() -> u32 {
            5
        }
    }

    assert_eq!(<(Impl, Impl, Impl)>::plus(), 15);
    assert_eq!(<(Impl, Impl, Impl)>::star(), 125);
    assert_eq!(<(Impl, Impl, Impl)>::minus(), -1000);
}

#[test]
fn semi_automatic_tuple_with_custom_trait_bound() {
    #[allow(dead_code)]
    trait Trait {
        type Arg;

        fn test(arg: Self::Arg);
    }

    trait Custom {
        type NewArg;

        fn new_test(arg: Self::NewArg);
    }

    #[impl_for_tuples(5)]
    #[tuple_types_custom_trait_bound(Custom)]
    impl Trait for Tuple {
        for_tuples!( type Arg = ( #( Tuple::NewArg ),* ); );
        fn test(arg: Self::Arg) {
            for_tuples!( #( Tuple::new_test(arg.Tuple); )* );
        }
    }
}

#[test]
fn semi_automatic_tuple_with_custom_advanced_trait_bound() {
    #[allow(dead_code)]
    trait Trait {
        type Arg;
        type Output;

        fn test(arg: Self::Arg);
        fn clone_test(&self) -> Self::Output;
    }

    trait Custom {
        type NewArg;

        fn new_test(arg: Self::NewArg);
    }

    #[impl_for_tuples(5)]
    #[tuple_types_custom_trait_bound(Custom + Clone)]
    impl Trait for Tuple {
        for_tuples!( type Arg = ( #( Tuple::NewArg ),* ); );
        for_tuples!( type Output = ( #( Tuple ),* ); );

        fn test(arg: Self::Arg) {
            for_tuples!( #( Tuple::new_test(arg.Tuple); )* );
        }

        fn clone_test(&self) -> Self::Output {
            for_tuples!( ( #( Tuple.clone() ),* ) );
        }
    }
}

#[test]
fn semi_automatic_tuple_as_ref() {
    trait Trait {
        type Arg;

        fn test(arg: Self::Arg);
    }

    #[impl_for_tuples(5)]
    impl Trait for &Tuple {
        for_tuples!( type Arg = ( #( Tuple::Arg ),* ); );
        fn test(arg: Self::Arg) {
            for_tuples!( #( Tuple::test(arg.Tuple); )* )
        }
    }
}

#[test]
fn semi_automatic_associated_const() {
    trait Trait {
        const TYPE: &'static [u32];
    }

    trait Trait2 {
        const TYPE: u32;
    }

    #[impl_for_tuples(1, 5)]
    #[tuple_types_no_default_trait_bound]
    impl Trait for Tuple {
        for_tuples!( where #( Tuple: Trait2 )* );

        for_tuples!( const TYPE: &'static [u32] = &[ #( Tuple::TYPE ),* ]; );
    }
}

#[test]
fn semi_automatic_associated_const_calculation() {
    trait Trait {
        const TYPE: u32;
    }

    #[impl_for_tuples(1, 5)]
    impl Trait for Tuple {
        for_tuples!( const TYPE: u32 = #( Tuple::TYPE )+*; );
    }

    struct Test;
    impl Trait for Test {
        const TYPE: u32 = 1;
    }

    assert_eq!(3, <(Test, Test, Test)>::TYPE);
    assert_eq!(1, Test::TYPE);
}

#[test]
fn semi_automatic_associated_type() {
    trait Trait {
        type A;
        type B;
    }

    #[impl_for_tuples(5)]
    impl Trait for Tuple {
        for_tuples!( type A = ( #( Option< Tuple::B > ),* ); );
        for_tuples!( type B = ( #( Tuple::B ),* ); );
    }
}

#[test]
fn semi_automatic_unsafe_trait() {
    unsafe trait Trait {
        type A;
    }

    #[impl_for_tuples(5)]
    unsafe impl Trait for Tuple {
        for_tuples!( type A = ( #( Tuple::A ),* ); );
    }
}
