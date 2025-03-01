//! Test ensuring that default type params gets forwarded to Builder.

#[macro_use]
extern crate derive_builder;

#[derive(Builder)]
#[builder(setter(strip_option))]
struct Settings<T, U = (), C = fn(T) -> U> {
    first: T,
    #[builder(default)]
    second: Option<U>,
    #[builder(default)]
    third: Option<C>,
}

fn main() {
    SettingsBuilder::<usize>::default()
        .first(1)
        .second(())
        .third(|_: usize| ())
        .build()
        .unwrap();

    SettingsBuilder::<usize, usize>::default()
        .first(1)
        .second(2)
        .third(|_: usize| 3)
        .build()
        .unwrap();
}
