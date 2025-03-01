use serde::Deserialize;
use figment::{Figment, providers::{Toml, Format}};

#[derive(Debug, Deserialize, PartialEq)]
struct Config {
    bs: Vec<bool>,
    u8s: Vec<u8>,
    i32s: Vec<i32>,
    f64s: Vec<f64>,
}

static TOML: &str = r##"
    u8s = [1, 2, 3, "4", 5, "6"]
    i32s = [-1, -2, 3, "-4", 5, "6"]
    f64s = [1, "2", -3, -4.5, "5.0", "-6.0"]
    bs = [true, false, "true", "false", "YES", "no", "on", "OFF", "1", "0", 1, 0]
"##;

#[test]
fn lossy_values() {
    let config: Config = Figment::from(Toml::string(TOML)).extract_lossy().unwrap();
    assert_eq!(&config.u8s, &[ 1, 2, 3, 4, 5, 6 ]);
    assert_eq!(&config.i32s, &[-1, -2, 3, -4, 5, 6]);
    assert_eq!(&config.f64s, &[1.0, 2.0, -3.0, -4.5, 5.0, -6.0]);
    assert_eq!(&config.bs, &[
        true, false, true, false, true, false, true, false, true, false, true, false
    ]);
}
