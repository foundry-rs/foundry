use serde::Deserialize;
use figment::{Figment, providers::{Format, Toml, Json, Env}};

#[test]
fn mini_cargo() {
    #[derive(Debug, PartialEq, Deserialize)]
    struct Package {
        name: String,
        description: Option<String>,
        authors: Vec<String>,
        publish: Option<bool>,
    }

    #[derive(Debug, PartialEq, Deserialize)]
    struct Config {
        package: Package,
        rustc: Option<String>,
        rustdoc: Option<String>,
    }

    // Replicate part of Cargo's config but also support `Cargo.json` with lower
    // precedence than `Cargo.toml`.
    figment::Jail::expect_with(|jail| {
        jail.create_file("Cargo.toml", r#"
            [package]
            name = "test"
            authors = ["bob"]
            publish = false
        "#)?;

        let config: Config = Figment::new()
            .merge(Toml::file("Cargo.toml"))
            .merge(Env::prefixed("CARGO_"))
            .merge(Env::raw().only(&["RUSTC", "RUSTDOC"]))
            .join(Json::file("Cargo.json"))
            .extract()?;

        assert_eq!(config, Config {
            package: Package {
                name: "test".into(),
                description: None,
                authors: vec!["bob".into()],
                publish: Some(false)
            },
            rustc: None,
            rustdoc: None
        });

        Ok(())
    });
}
