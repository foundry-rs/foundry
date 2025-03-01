use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::{
    common::{APDUAnswer, APDUCommand},
    errors::LedgerError,
};

// Compilation would fail either way, since the following `extern "C"` block
// would not be linked to anything
#[cfg(not(any(feature = "node", feature = "browser")))]
compile_error!("Either `node` or `browser` feature must be enabled for WASM transport");

// These conditional compilation blocks ensure that we try to import the correct
// transport for our environment.
#[cfg_attr(
    feature = "node",
    wasm_bindgen(module = "@ledgerhq/hw-transport-node-hid")
)]
#[cfg_attr(
    feature = "browser",
    wasm_bindgen(module = "@ledgerhq/hw-transport-webusb")
)]
extern "C" {
    // NB:
    // This causes the JS glue to bind the variable `default1`
    // This took hours to figure out -_-
    #[allow(non_camel_case_types)]
    pub type default;

    #[wasm_bindgen(static_method_of = default)]
    fn create() -> js_sys::Promise;
}

#[wasm_bindgen]
extern "C" {
    pub type Transport;

    // `transport.exchange(apdu: Buffer): Promise<Buffer>`
    //
    // See [here](https://github.com/LedgerHQ/ledgerjs#an-unified-transport-interface)
    #[wasm_bindgen(method)]
    fn exchange(t: &Transport, buf: &[u8]) -> js_sys::Promise;
}

/// Transport struct for non-wasm arch
#[wasm_bindgen]
pub struct LedgerTransport(Transport);

impl std::fmt::Debug for LedgerTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        #[cfg(feature = "node")]
        let transport_type = "@ledgerhq/hw-transport-node-hid";
        #[cfg(feature = "browser")]
        let transport_type = "@ledgerhq/hw-transport-webusb";

        f.debug_struct("LedgerTransport")
            .field("transport_library", &transport_type)
            .finish()
    }
}

/// Transport Impl for wasm
impl LedgerTransport {
    /// Send an APDU command to the device, and receive a response
    pub async fn exchange(&self, apdu_command: &APDUCommand) -> Result<APDUAnswer, LedgerError> {
        let promise = self.0.exchange(&apdu_command.serialize());

        let future = JsFuture::from(promise);

        // Transport Error
        let result = future
            .await
            .map_err(|e| LedgerError::JsError(format!("{:?}", &e)))?;
        let answer = js_sys::Uint8Array::new(&result).to_vec();

        APDUAnswer::from_answer(answer)
    }
}

#[wasm_bindgen]
impl LedgerTransport {
    /// Instantiate a new transport by calling `create` on the JS `@ledgerhq/hw-transport-*` mod
    pub async fn create() -> Result<LedgerTransport, JsValue> {
        let fut = JsFuture::from(default::create());
        let transport: Transport = fut.await?.into();
        Ok(Self(transport))
    }

    /// Instantiate from a js transport object
    #[allow(clippy::missing_const_for_fn)] // not allowed on wasm bindgen fns
    pub fn from_js_transport(transport: Transport) -> Self {
        Self(transport)
    }

    #[doc(hidden)]
    // NB: this invalidates the JS ref to the wasm and makes the object unusable.
    pub async fn debug_send(self) -> Result<js_sys::Uint8Array, JsValue> {
        let command_buf: &[u8] = &[];

        // Ethereum `get_app_version`
        let command = APDUCommand {
            cla: 0xe0,
            ins: 0x06,
            p1: 0x00,
            p2: 0x00,
            data: command_buf.into(),
            response_len: None,
        };

        let answer = self
            .exchange(&command)
            .await
            .map_err(|e| JsValue::from_str(&format!("{:?}", e)))?;
        let payload = answer.data().unwrap_or(&[]);
        Ok(js_sys::Uint8Array::from(payload))
    }
}

/*******************************************************************************
*   (c) 2020 ZondaX GmbH
*
*  Licensed under the Apache License, Version 2.0 (the "License");
*  you may not use this file except in compliance with the License.
*  You may obtain a copy of the License at
*
*      http://www.apache.org/licenses/LICENSE-2.0
*
*  Unless required by applicable law or agreed to in writing, software
*  distributed under the License is distributed on an "AS IS" BASIS,
*  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
*  See the License for the specific language governing permissions and
*  limitations under the License.
********************************************************************************/
