use serde::Deserialize;
use figment::{Figment, providers::{Format, Yaml}};

#[derive(Deserialize, PartialEq, Debug, Clone)]
pub enum Trigger {
    Start,
    Inter { sec: u32 },
    Triple(usize, usize, usize),
    End(String),
}

#[derive(Deserialize, Debug, Clone)]
pub struct Script {
    pub triggers: Vec<Trigger>,
}

const YAML: &str = "
triggers:
  - !Start
  - !Inter
      sec: 5
  - !Inter { sec: 7, }
  - !Triple [1, 2, 3]
  - !End Now
";

const YAML2: &str = "
triggers:
  - Start:
  - Inter:
      sec: 5
  - !Inter { sec: 7, }
  - Triple:
    - 1
    - 2
    - 3
  - End: Now
";

#[test]
fn figment_yaml_deserize() {
    let figment = Figment::new().merge(Yaml::string(YAML));
    let script = figment.extract::<Script>().unwrap();
    assert_eq!(script.triggers[0], Trigger::Start);
    assert_eq!(script.triggers[1], Trigger::Inter { sec: 5 });
    assert_eq!(script.triggers[2], Trigger::Inter { sec: 7 });
    assert_eq!(script.triggers[3], Trigger::Triple(1, 2, 3));
    assert_eq!(script.triggers[4], Trigger::End("Now".into()));

    let figment = Figment::new().merge(Yaml::string(YAML2));
    let script = figment.extract::<Script>().unwrap();
    assert_eq!(script.triggers[0], Trigger::Start);
    assert_eq!(script.triggers[1], Trigger::Inter { sec: 5 });
    assert_eq!(script.triggers[2], Trigger::Inter { sec: 7 });
    assert_eq!(script.triggers[3], Trigger::Triple(1, 2, 3));
    assert_eq!(script.triggers[4], Trigger::End("Now".into()));
}
