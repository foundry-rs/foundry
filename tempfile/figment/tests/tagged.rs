use figment::{Figment, Jail, Profile};
use figment::{value::{Value, magic::Tagged}, providers::Serialized};

#[test]
fn check_values_are_tagged_with_profile() {
    Jail::expect_with(|_| {
        let figment = Figment::new()
            .merge(Serialized::default("default", "default"))
            .merge(Serialized::global("global", "global"))
            .merge(Serialized::default("custom", "custom").profile("custom"));

        let tagged: Tagged<String> = figment.extract_inner("default")?;
        let value: Value = figment.find_value("default")?;
        assert_eq!(tagged.tag().profile(), Some(Profile::Default));
        assert_eq!(value.tag().profile(), Some(Profile::Default));

        let tagged: Tagged<String> = figment.extract_inner("global")?;
        let value: Value = figment.find_value("global")?;
        assert_eq!(tagged.tag().profile(), Some(Profile::Global));
        assert_eq!(value.tag().profile(), Some(Profile::Global));

        let figment = figment.select("custom");
        let tagged: Tagged<String> = figment.extract_inner("custom")?;
        let value: Value = figment.find_value("custom")?;
        assert_eq!(tagged.tag().profile(), None);
        assert_eq!(value.tag().profile(), None);

        Ok(())
    });
}

#[test]
fn check_errors_are_tagged_with_path() {
    Jail::expect_with(|_| {
        let figment = Figment::new()
            .merge(("foo", 123))
            .merge(("foo.bar", 789))
            .merge(("baz", 789));

        let err = figment.extract_inner::<String>("foo").unwrap_err();
        assert_eq!(err.path, vec!["foo"]);

        let err = figment.extract_inner::<String>("foo.bar").unwrap_err();
        assert_eq!(err.path, vec!["foo", "bar"]);

        let err = figment.extract_inner::<usize>("foo.bar.baz").unwrap_err();
        assert!(err.path.is_empty());
        Ok(())
    });
}
