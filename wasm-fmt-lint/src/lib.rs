use wasm_bindgen::prelude::*;

// Re-export console for debugging if needed
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

// #[wasm_bindgen]
// pub fn __wasm_start() {
//     // Intentionally left blank; exported to satisfy wasmbuild's startup call.
// }

#[derive(serde::Deserialize, serde::Serialize)]
pub struct FormatResult {
    pub formatted: String,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct ErrorResult {
    pub error: String,
}

#[wasm_bindgen]
pub fn fmt_default(source: &str) -> Result<JsValue, JsValue> {
    let res = forge_fmt::format(source)
        .map(|formatted| serde_wasm_bindgen::to_value(&FormatResult { formatted }).unwrap())
        .map_err(|e| serde_wasm_bindgen::to_value(&ErrorResult { error: e.to_string() }).unwrap());
    res
}

#[wasm_bindgen]
pub fn fmt_with_config(source: &str, config: JsValue) -> Result<JsValue, JsValue> {
    use forge_fmt::{FormatterConfig, format_to, parse};

    let cfg: FormatterConfig = if config.is_undefined() || config.is_null() {
        FormatterConfig::default()
    } else {
        match serde_wasm_bindgen::from_value::<FormatterConfig>(config) {
            Ok(v) => v,
            Err(e) => {
                return Err(serde_wasm_bindgen::to_value(&ErrorResult {
                    error: format!("Invalid config: {e}"),
                })
                .unwrap());
            }
        }
    };

    match parse(source) {
        Ok(parsed) => {
            let mut out = String::new();
            if let Err(e) = format_to(&mut out, parsed, cfg) {
                Err(serde_wasm_bindgen::to_value(&ErrorResult { error: e.to_string() }).unwrap())
            } else {
                Ok(serde_wasm_bindgen::to_value(&FormatResult { formatted: out }).unwrap())
            }
        }
        Err(e) => Err(serde_wasm_bindgen::to_value(&ErrorResult { error: e.to_string() }).unwrap()),
    }
}

#[wasm_bindgen]
pub fn fmt_config_default() -> JsValue {
    let cfg = forge_fmt::FormatterConfig::default();
    serde_wasm_bindgen::to_value(&cfg).unwrap()
}

// Placeholder Lint API (to be implemented once browser threading and deps are aligned)
#[derive(serde::Deserialize, serde::Serialize)]
pub struct LintOptions {
    /// Future: severity filters, include/exclude, mixed_case_exceptions, etc.
    #[serde(default)]
    pub severity: Vec<String>,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct LintResult {
    pub diagnostics: Vec<serde_json::Value>,
    pub note: Option<String>,
}

#[wasm_bindgen]
pub fn lint(_source: &str, _options: JsValue) -> Result<JsValue, JsValue> {
    let res = LintResult {
        diagnostics: vec![],
        note: Some("lint is not yet available in the wasm build. This stub exists to keep the API stable while we upstream wasm support in forge-lint.".to_string()),
    };
    Ok(serde_wasm_bindgen::to_value(&res).unwrap())
}
