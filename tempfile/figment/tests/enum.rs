use serde::{Deserialize, Serialize};
use figment::{Figment, providers::{Format, Toml, Serialized, Env}};

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Config {
    foo: Option<Foo>,
    bar: Option<Bar>,
    baz: Option<Baz>,
}

#[derive(PartialEq, Debug, Deserialize, Serialize)]
pub enum Foo {
    Mega,
    Supa
}

#[derive(PartialEq, Debug, Deserialize, Serialize)]
pub enum Bar {
    None,
    Some(usize, String)
}

#[derive(PartialEq, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Baz {
    A(String),
    B(usize),
}

#[test]
fn test_enum_de() {
    let figment = || Figment::new()
        .merge(Serialized::defaults(Config {
            foo: None,
            bar: Some(Bar::Some(9999, "not-the-string".into())),
            baz: None
        }))
        .merge(Toml::file("Test.toml"))
        .merge(Env::prefixed("TEST_"));

    figment::Jail::expect_with(|jail| {
        let test: Config = figment().extract()?;
        assert_eq!(test.foo, None);
        assert_eq!(test.bar, Some(Bar::Some(9999, "not-the-string".into())));
        assert_eq!(test.baz, None);

        jail.create_file("Test.toml", r#"
            foo = "Mega"
            baz = "goobar"

            [bar]
            Some = [10, "hi"]
        "#)?;

        let test: Config = figment().extract()?;
        assert_eq!(test.foo, Some(Foo::Mega));
        assert_eq!(test.bar, Some(Bar::Some(10, "hi".into())));
        assert_eq!(test.baz, Some(Baz::A("goobar".into())));

        jail.set_env("TEST_foo", "Supa");
        jail.set_env("TEST_bar", "None");

        let test: Config = figment().extract()?;
        assert_eq!(test.foo, Some(Foo::Supa));
        assert_eq!(test.bar, Some(Bar::None));

        Ok(())
    })
}
