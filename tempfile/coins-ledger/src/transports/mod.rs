//! Abstract ledger tranport trait with WASM and native HID instantiations.

use crate::{
    common::{APDUAnswer, APDUCommand},
    errors::LedgerError,
};
use async_trait::async_trait;

cfg_if::cfg_if! {
    if #[cfg(target_arch = "wasm32")] {
        /// APDU Transport wrapper for JS/WASM transports.
        pub mod wasm;
        pub use wasm::LedgerTransport as DefaultTransport;

        use log::{debug, error};
    } else {
        /// APDU Transport for native HID. Wraps
        /// [`TransportNativeHID`][native::hid::TransportNativeHID] in a
        /// thread managing the IO.
        pub mod native;
        pub use native::LedgerHandle as DefaultTransport;

        use tracing::{debug, error};
    }
}

/// A Ledger device connection. This wraps the default transport type. In
/// native code, this is the `hidapi` library. When the `node` or `browser`
/// feature is selected, it is a Ledger JS transport library.
#[derive(Debug)]
pub struct Ledger(DefaultTransport);

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
/// An asynchronous interface to the Ledger device. It is critical that the
/// device have only one connection active, so the `init` funnction acquires a
/// lock on the device.
pub trait LedgerAsync: Sized {
    /// Init the connection to the device. This may fail if the device is already in use by some
    /// other process.
    async fn init() -> Result<Self, LedgerError>;

    /// Exchange a packet with the device.
    async fn exchange(&self, packet: &APDUCommand) -> Result<APDUAnswer, LedgerError>;

    /// Consume the connection, and release the resources it holds.
    ///
    /// By default this function simply drops the struct.
    fn close(self) {}
}

#[cfg(not(target_arch = "wasm32"))]
#[async_trait]
impl LedgerAsync for Ledger {
    async fn init() -> Result<Self, LedgerError> {
        Ok(Self(DefaultTransport::init()?))
    }

    async fn exchange(&self, packet: &APDUCommand) -> Result<APDUAnswer, LedgerError> {
        debug!(command = %packet, "dispatching APDU to device");

        // TODO: remove clone
        let resp = self.0.exchange(packet.clone()).await;
        match &resp {
            Ok(resp) => {
                debug!(
                    retcode = resp.retcode(),
                    response = resp.data().map(hex::encode),
                    "Received response from device"
                )
            }
            Err(err) => error!(%err, "Error during communication"),
        }
        resp
    }
}

#[cfg(target_arch = "wasm32")]
#[async_trait(?Send)]
impl LedgerAsync for Ledger {
    async fn init() -> Result<Self, LedgerError> {
        let res: Result<DefaultTransport, wasm_bindgen::JsValue> = DefaultTransport::create().await;
        let res: Result<DefaultTransport, LedgerError> = res.map_err(|err| err.into());
        Ok(Self(res?))
    }

    async fn exchange(&self, packet: &APDUCommand) -> Result<APDUAnswer, LedgerError> {
        debug!("Exchanging Packet {:#?}", packet);
        let resp = self.0.exchange(packet).await;
        match &resp {
            Ok(resp) => debug!("Got response: {:#?}", &resp),
            Err(e) => error!("Got error: {}", e),
        }
        resp
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
