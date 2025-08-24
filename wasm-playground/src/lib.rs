use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn error(s: &str);
}

#[wasm_bindgen]
pub fn calldata_encode(signature: &str, args: JsValue) -> Result<String, JsValue> {
    use alloy_dyn_abi::{DynSolType, DynSolValue};
    use alloy_primitives::{hex, utils::keccak256};

    // Parse signature
    let (name, params) =
        signature.split_once('(').ok_or_else(|| JsValue::from_str("invalid function signature"))?;
    let params = params.strip_suffix(')').unwrap_or(params);
    let type_strs: Vec<&str> = if params.trim().is_empty() {
        vec![]
    } else {
        params.split(',').map(|s| s.trim()).collect()
    };

    // Parse types and args
    let types: Vec<DynSolType> = type_strs
        .iter()
        .map(|t| DynSolType::parse(t).map_err(|e| JsValue::from_str(&format!("{e}"))))
        .collect::<Result<_, _>>()?;
    let args_vec: Vec<String> = serde_wasm_bindgen::from_value(args)
        .map_err(|e| JsValue::from_str(&format!("invalid args array: {e}")))?;
    if args_vec.len() != types.len() {
        return Err(JsValue::from_str("argument count mismatch"));
    }
    let values: Vec<DynSolValue> = types
        .iter()
        .zip(args_vec.iter())
        .map(|(ty, s)| {
            DynSolType::coerce_str(ty, s).map_err(|e| JsValue::from_str(&format!("{e}")))
        })
        .collect::<Result<_, _>>()?;

    // Encode
    let selector_sig = format!("{name}({})", type_strs.join(","));
    let mut out = Vec::with_capacity(4 + 32 * values.len());
    out.extend_from_slice(&keccak256(selector_sig.as_bytes())[..4]);
    let data = DynSolValue::Tuple(values).abi_encode();
    out.extend_from_slice(&data);
    Ok(format!("0x{}", hex::encode(out)))
}

#[wasm_bindgen]
pub async fn rpc(url: String, method: String, params: JsValue) -> Result<JsValue, JsValue> {
    use js_sys::Promise;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Headers, Request, RequestInit, Response};

    // Prepare JSON-RPC body
    let params_js = params;
    let body = js_sys::Object::new();
    js_sys::Reflect::set(&body, &JsValue::from_str("jsonrpc"), &JsValue::from_str("2.0")).unwrap();
    js_sys::Reflect::set(&body, &JsValue::from_str("id"), &JsValue::from_f64(1.0)).unwrap();
    js_sys::Reflect::set(&body, &JsValue::from_str("method"), &JsValue::from_str(&method)).unwrap();
    js_sys::Reflect::set(&body, &JsValue::from_str("params"), &params_js).unwrap();
    let body_str = js_sys::JSON::stringify(&body)
        .map_err(|e| JsValue::from_str(&format!("failed to stringify body: {e:?}")))?
        .as_string()
        .ok_or_else(|| JsValue::from_str("failed to stringify body"))?;

    // Build request
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_mode(web_sys::RequestMode::Cors);
    init.set_body(&JsValue::from_str(&body_str));
    let request = Request::new_with_str_and_init(&url, &init)
        .map_err(|e| JsValue::from_str(&format!("failed to build request: {e:?}")))?;
    let headers = Headers::new().unwrap();
    headers.set("Content-Type", "application/json").unwrap();
    request.headers().set("Content-Type", "application/json").unwrap();

    // Fetch
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let resp_value = JsFuture::from(Promise::from(window.fetch_with_request(&request)))
        .await
        .map_err(|e| JsValue::from_str(&format!("fetch failed: {e:?}")))?;
    let resp: Response = resp_value.dyn_into().unwrap();
    if !resp.ok() {
        let status = resp.status();
        return Err(JsValue::from_str(&format!("HTTP error {status}")));
    }
    let json = JsFuture::from(resp.json().unwrap()).await.unwrap();
    Ok(json)
}
