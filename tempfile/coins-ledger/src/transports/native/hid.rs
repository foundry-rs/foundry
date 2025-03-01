//! Native HID APDU transport for Ledger Nano hardware wallets

use crate::{
    common::{APDUAnswer, APDUCommand},
    errors::LedgerError,
};

use byteorder::{BigEndian, ReadBytesExt};
use hidapi_rusb::{DeviceInfo, HidApi, HidDevice};
use once_cell::sync::Lazy;
use std::{
    io::Cursor,
    sync::{Mutex, MutexGuard},
};

use super::NativeTransportError;

const LEDGER_VID: u16 = 0x2c97;
#[cfg(not(target_os = "linux"))]
const LEDGER_USAGE_PAGE: u16 = 0xFFA0;
const LEDGER_CHANNEL: u16 = 0x0101;
// for Windows compatability, we prepend the buffer with a 0x00
// so the actual buffer is 64 bytes
const LEDGER_PACKET_WRITE_SIZE: u8 = 65;
const LEDGER_PACKET_READ_SIZE: u8 = 64;
const LEDGER_TIMEOUT: i32 = 10_000_000;

/// The HID API instance.
pub static HIDAPI: Lazy<HidApi> =
    Lazy::new(|| HidApi::new().expect("Failed to initialize HID API"));

/// Native HID transport for Ledger Nano hardware wallets
pub struct TransportNativeHID {
    device: Mutex<HidDevice>,
}

impl std::fmt::Debug for TransportNativeHID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransportNativeHID").finish()
    }
}

#[cfg(not(target_os = "linux"))]
fn is_ledger(dev: &DeviceInfo) -> bool {
    dev.vendor_id() == LEDGER_VID && dev.usage_page() == LEDGER_USAGE_PAGE
}

#[cfg(target_os = "linux")]
fn is_ledger(dev: &DeviceInfo) -> bool {
    dev.vendor_id() == LEDGER_VID
}

/// Get a list of ledger devices available
fn list_ledgers(api: &HidApi) -> impl Iterator<Item = &DeviceInfo> {
    api.device_list().filter(|dev| is_ledger(dev))
}

#[tracing::instrument(skip_all, err)]
fn first_ledger(api: &HidApi) -> Result<HidDevice, NativeTransportError> {
    let device = list_ledgers(api)
        .next()
        .ok_or(NativeTransportError::DeviceNotFound)?;

    open_device(api, device)
}

/// Read the 5-byte response header.
fn read_response_header(rdr: &mut Cursor<&[u8]>) -> Result<(u16, u8, u16), NativeTransportError> {
    let rcv_channel = rdr.read_u16::<BigEndian>()?;
    let rcv_tag = rdr.read_u8()?;
    let rcv_seq_idx = rdr.read_u16::<BigEndian>()?;
    Ok((rcv_channel, rcv_tag, rcv_seq_idx))
}

fn write_apdu(
    device: &mut MutexGuard<'_, HidDevice>,
    channel: u16,
    apdu_command: &[u8],
) -> Result<(), NativeTransportError> {
    tracing::debug!(apdu = %hex::encode(apdu_command), bytes = apdu_command.len(), "Writing APDU to device");

    let command_length = apdu_command.len();

    // TODO: allocation-free method
    let mut in_data = Vec::with_capacity(command_length + 2);
    in_data.push(((command_length >> 8) & 0xFF) as u8);
    in_data.push((command_length & 0xFF) as u8);
    in_data.extend_from_slice(apdu_command);

    let mut buffer = [0u8; LEDGER_PACKET_WRITE_SIZE as usize];
    // Windows platform requires 0x00 prefix and Linux/Mac tolerate this as
    // well. So we leave buffer[0] as 0x00
    buffer[1] = ((channel >> 8) & 0xFF) as u8; // channel big endian
    buffer[2] = (channel & 0xFF) as u8; // channel big endian
    buffer[3] = 0x05u8;

    for (sequence_idx, chunk) in in_data
        .chunks((LEDGER_PACKET_WRITE_SIZE - 6) as usize)
        .enumerate()
    {
        buffer[4] = ((sequence_idx >> 8) & 0xFF) as u8; // sequence_idx big endian
        buffer[5] = (sequence_idx & 0xFF) as u8; // sequence_idx big endian
        buffer[6..6 + chunk.len()].copy_from_slice(chunk);

        tracing::trace!(
            buffer = hex::encode(buffer),
            sequence_idx,
            bytes = chunk.len(),
            "Writing chunk to device",
        );
        let result = device.write(&buffer).map_err(NativeTransportError::Hid)?;
        if result < buffer.len() {
            return Err(NativeTransportError::Comm(
                "USB write error. Could not send whole message",
            ));
        }
    }
    Ok(())
}

/// Read a response APDU from the ledger channel.
fn read_response_apdu(
    device: &mut MutexGuard<'_, HidDevice>,
    _channel: u16,
) -> Result<Vec<u8>, NativeTransportError> {
    let mut response_buffer = [0u8; LEDGER_PACKET_READ_SIZE as usize];
    let mut sequence_idx = 0u16;
    let mut expected_response_len = 0usize;
    let mut offset = 0;

    let mut answer_buf = vec![];

    loop {
        let remaining = expected_response_len
            .checked_sub(offset)
            .unwrap_or_default();

        tracing::trace!(
            sequence_idx,
            expected_response_len,
            remaining,
            answer_size = answer_buf.len(),
            "Reading response from device.",
        );

        let res = device.read_timeout(&mut response_buffer, LEDGER_TIMEOUT)?;

        // The first packet contains the response length as u16, successive
        // packets do not.
        if (sequence_idx == 0 && res < 7) || res < 5 {
            return Err(NativeTransportError::Comm("Read error. Incomplete header"));
        }

        let mut rdr: Cursor<&[u8]> = Cursor::new(&response_buffer[..]);
        let (_, _, rcv_seq_idx) = read_response_header(&mut rdr)?;

        // Check sequence index. A mismatch means someone else read a packet.s
        if rcv_seq_idx != sequence_idx {
            return Err(NativeTransportError::SequenceMismatch {
                got: rcv_seq_idx,
                expected: sequence_idx,
            });
        }

        // The header packet contains the number of bytes of response data
        if rcv_seq_idx == 0 {
            expected_response_len = rdr.read_u16::<BigEndian>()? as usize;
            tracing::trace!(
                expected_response_len,
                "Received response length from device",
            );
        }

        // Read either until the end of the buffer, or until we have read the
        // expected response length
        let remaining_in_buf = response_buffer.len() - rdr.position() as usize;
        let missing = expected_response_len - offset;
        let end_p = rdr.position() as usize + std::cmp::min(remaining_in_buf, missing);

        let new_chunk = &response_buffer[rdr.position() as usize..end_p];

        // Copy the response to the answer
        answer_buf.extend(new_chunk);
        offset += new_chunk.len();

        if offset >= expected_response_len {
            return Ok(answer_buf);
        }

        sequence_idx += 1;
    }
}

/// Open a specific ledger device
///
/// # Note
/// No checks are made to ensure the device is a ledger device
///
/// # Warning
/// Opening the same device concurrently will lead to device lock after the first handle is closed
/// see [issue](https://github.com/ruabmbua/hidapi-rs/issues/81)
fn open_device(api: &HidApi, device: &DeviceInfo) -> Result<HidDevice, NativeTransportError> {
    let device = device
        .open_device(api)
        .map_err(NativeTransportError::CantOpen)?;
    let _ = device.set_blocking_mode(true);

    Ok(device)
}

impl TransportNativeHID {
    /// Instantiate from a device.
    const fn from_device(device: HidDevice) -> Self {
        Self {
            device: Mutex::new(device),
        }
    }

    /// Open all ledger devices.
    pub fn open_all_devices() -> Result<Vec<Self>, NativeTransportError> {
        let api = &HIDAPI;
        let devices = list_ledgers(api)
            .map(|dev| open_device(api, dev))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(devices.into_iter().map(Self::from_device).collect())
    }

    /// Create a new HID transport, connecting to the first ledger found
    ///
    /// # Warning
    /// Opening the same device concurrently will lead to device lock after the first handle is closed
    /// see [issue](https://github.com/ruabmbua/hidapi-rs/issues/81)
    pub fn new() -> Result<Self, NativeTransportError> {
        let api = &HIDAPI;

        #[cfg(target_os = "android")]
        {
            // Using runtime detection since it's impossible to statically target Termux.
            let is_termux = match std::env::var("PREFIX") {
                Ok(prefix_var) => prefix_var.contains("/com.termux/"),
                Err(_) => false,
            };

            if is_termux {
                // Termux uses a special environment vairable TERMUX_USB_FD for this
                let usb_fd = std::env::var("TERMUX_USB_FD")
                    .map_err(|_| NativeTransportError::InvalidTermuxUsbFd)?
                    .parse::<i32>()
                    .map_err(|_| NativeTransportError::InvalidTermuxUsbFd)?;
                return Ok(api.wrap_sys_device(usb_fd, -1).map(Self::from_device)?);
            }
        }

        first_ledger(api).map(Self::from_device)
    }

    /// Get manufacturer string. Returns None on error, or on no string.
    pub fn get_manufacturer_string(&self) -> Option<String> {
        let device = self.device.lock().unwrap();
        device.get_manufacturer_string().unwrap_or_default()
    }

    /// Exchange an APDU with the device. The response data will be written to `answer_buf`, and a
    /// `APDUAnswer` struct will be created with a reference to `answer_buf`.
    ///
    /// It is strongly recommended that you use the `APDUAnswer` api instead of reading the raw
    /// answer_buf response.
    ///
    /// If the method errors, the buf may contain a partially written response. It is not advised
    /// to read this.
    pub fn exchange(&self, command: &APDUCommand) -> Result<APDUAnswer, LedgerError> {
        let answer = {
            let mut device = self.device.lock().unwrap();
            write_apdu(&mut device, LEDGER_CHANNEL, &command.serialize())?;
            read_response_apdu(&mut device, LEDGER_CHANNEL)?
        };

        let answer = APDUAnswer::from_answer(answer)?;

        match answer.response_status() {
            None => Ok(answer),
            Some(response) => {
                if response.is_success() {
                    Ok(answer)
                } else {
                    Err(response.into())
                }
            }
        }
    }
}

/*******************************************************************************
*   (c) 2018-2022 ZondaX GmbH
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
