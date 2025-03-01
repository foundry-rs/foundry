use serde_json::value::Value as Json;

use crate::context::Context;
use crate::error::RenderError;
use crate::helpers::HelperDef;
use crate::json::value::ScopedJson;
use crate::registry::Registry;
use crate::render::{Helper, RenderContext};
use crate::RenderErrorReason;

#[derive(Clone, Copy)]
pub struct LookupHelper;

impl HelperDef for LookupHelper {
    fn call_inner<'reg: 'rc, 'rc>(
        &self,
        h: &Helper<'rc>,
        r: &'reg Registry<'reg>,
        _: &'rc Context,
        _: &mut RenderContext<'reg, 'rc>,
    ) -> Result<ScopedJson<'rc>, RenderError> {
        let collection_value = h
            .param(0)
            .ok_or(RenderErrorReason::ParamNotFoundForIndex("lookup", 0))?;
        let index = h
            .param(1)
            .ok_or(RenderErrorReason::ParamNotFoundForIndex("lookup", 1))?;

        let value = match *collection_value.value() {
            Json::Array(ref v) => index.value().as_u64().and_then(|u| v.get(u as usize)),
            Json::Object(ref m) => index.value().as_str().and_then(|k| m.get(k)),
            _ => None,
        };
        if r.strict_mode() && value.is_none() {
            Err(RenderError::strict_error(None))
        } else {
            Ok(value.unwrap_or(&Json::Null).clone().into())
        }
    }
}

pub static LOOKUP_HELPER: LookupHelper = LookupHelper;

#[cfg(test)]
mod test {
    use crate::registry::Registry;

    #[test]
    fn test_lookup() {
        let mut handlebars = Registry::new();
        assert!(handlebars
            .register_template_string("t0", "{{#each v1}}{{lookup ../v2 @index}}{{/each}}")
            .is_ok());
        assert!(handlebars
            .register_template_string("t1", "{{#each v1}}{{lookup ../v2 1}}{{/each}}")
            .is_ok());
        assert!(handlebars
            .register_template_string("t2", "{{lookup kk \"a\"}}")
            .is_ok());

        let m = json!({"v1": [1,2,3], "v2": [9,8,7]});

        let m2 = json!({
            "kk": {"a": "world"}
        });

        let r0 = handlebars.render("t0", &m);
        assert_eq!(r0.ok().unwrap(), "987".to_string());

        let r1 = handlebars.render("t1", &m);
        assert_eq!(r1.ok().unwrap(), "888".to_string());

        let r2 = handlebars.render("t2", &m2);
        assert_eq!(r2.ok().unwrap(), "world".to_string());

        assert!(handlebars.render_template("{{lookup}}", &m).is_err());
        assert!(handlebars.render_template("{{lookup v1}}", &m).is_err());
        assert_eq!(
            handlebars.render_template("{{lookup null 1}}", &m).unwrap(),
            ""
        );
        assert_eq!(
            handlebars.render_template("{{lookup v1 3}}", &m).unwrap(),
            ""
        );
    }

    #[test]
    fn test_strict_lookup() {
        let mut hbs = Registry::new();

        assert_eq!(
            hbs.render_template("{{lookup kk 1}}", &json!({"kk": []}))
                .unwrap(),
            ""
        );
        assert!(hbs
            .render_template("{{lookup kk 0}}", &json!({ "kk": [null] }))
            .is_ok());

        hbs.set_strict_mode(true);

        assert!(hbs
            .render_template("{{lookup kk 1}}", &json!({"kk": []}))
            .is_err());
        assert!(hbs
            .render_template("{{lookup kk 0}}", &json!({ "kk": [null] }))
            .is_ok());
    }
}
