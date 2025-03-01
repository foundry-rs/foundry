use figment::{Figment, providers::Env};

#[test]
fn camel_case() {
    #[derive(serde::Deserialize, PartialEq, Debug)]
    #[serde(rename_all = "camelCase")]
    struct Config {
        top_key_1: i32
    }

    figment::Jail::expect_with(|jail| {
        jail.set_env("topKey1", "100");

        let config: Config = Figment::new()
            .merge(Env::raw().lowercase(false))
            .extract()
            .unwrap();

        assert_eq!(config.top_key_1, 100);
        Ok(())
    });
}
