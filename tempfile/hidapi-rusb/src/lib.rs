// **************************************************************************
// Copyright (c) 2015 Osspial All Rights Reserved.
//
// This file is part of hidapi-rs, based on hidapi_rust by Roland Ruckerbauer.
// *************************************************************************

//! This crate provides a rust abstraction over the features of the C library
//! hidapi by [signal11](https://github.com/libusb/hidapi).
//!
//! # Usage
//!
//! This crate is [on crates.io](https://crates.io/crates/hidapi) and can be
//! used by adding `hidapi` to the dependencies in your project's `Cargo.toml`.
//!
//! # Example
//!
//! ```rust,no_run
//! extern crate hidapi_rusb;
//!
//! use hidapi_rusb::HidApi;
//!
//! fn main() {
//!     println!("Printing all available hid devices:");
//!
//!     match HidApi::new() {
//!         Ok(api) => {
//!             for device in api.device_list() {
//!                 println!("{:04x}:{:04x}", device.vendor_id(), device.product_id());
//!             }
//!         },
//!         Err(e) => {
//!             eprintln!("Error: {}", e);
//!         },
//!     }
//! }
//! ```

// Allow use of deprecated items, we defined ourselfes...
#![allow(deprecated)]

#[cfg(all(feature = "linux-static-rusb", not(target_os = "macos")))]
extern crate rusb;

extern crate libc;

mod error;
mod ffi;

use libc::{c_int, size_t, wchar_t};
use std::ffi::CStr;
use std::ffi::CString;
use std::fmt;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub use error::HidError;

pub type HidResult<T> = Result<T, HidError>;

const STRING_BUF_LEN: usize = 128;

/// Hidapi context and device member, which ensures deinitialization
/// of the C library happens, when, and only when all devices and the api instance is dropped.
struct HidApiLock;

impl HidApiLock {
    fn acquire() -> HidResult<HidApiLock> {
        const EXPECTED_CURRENT: bool = false;

        if EXPECTED_CURRENT
            == HID_API_LOCK.compare_and_swap(EXPECTED_CURRENT, true, Ordering::SeqCst)
        {
            // Initialize the HID and prevent other HIDs from being created
            unsafe {
                // This option must be set for Android Termux
                #[cfg(target_os = "android")]
                rusb::ffi::libusb_set_option(
                    std::ptr::null_mut(),
                    rusb::ffi::constants::LIBUSB_OPTION_WEAK_AUTHORITY,
                );

                if ffi::hid_init() == -1 {
                    HID_API_LOCK.store(false, Ordering::SeqCst);
                    return Err(HidError::InitializationError);
                }
                Ok(HidApiLock)
            }
        } else {
            Err(HidError::InitializationError)
        }
    }
}

impl Drop for HidApiLock {
    fn drop(&mut self) {
        unsafe {
            ffi::hid_exit();
        }
        HID_API_LOCK.store(false, Ordering::SeqCst);
    }
}

/// Object for handling hidapi context and implementing RAII for it.
/// Only one instance can exist at a time.
pub struct HidApi {
    devices: Vec<HidDeviceInfo>, /* Deprecated */
    device_list: Vec<DeviceInfo>,
    _lock: Arc<HidApiLock>,
}

static HID_API_LOCK: AtomicBool = AtomicBool::new(false);

impl HidApi {
    /// Initializes the hidapi.
    ///
    /// Will also initialize the currently available device list.
    pub fn new() -> HidResult<Self> {
        let lock = HidApiLock::acquire()?;

        let device_list = unsafe { HidApi::get_hid_device_info_vector()? };

        Ok(HidApi {
            device_list: device_list.clone(),
            devices: device_list.into_iter().map(|d| d.into()).collect(),
            _lock: Arc::new(lock),
        })
    }

    /// Refresh devices list and information about them (to access them use
    /// `device_list()` method)
    pub fn refresh_devices(&mut self) -> HidResult<()> {
        let device_list = unsafe { HidApi::get_hid_device_info_vector()? };
        self.device_list = device_list.clone();
        self.devices = device_list.into_iter().map(|d| d.into()).collect();
        Ok(())
    }

    unsafe fn get_hid_device_info_vector() -> HidResult<Vec<DeviceInfo>> {
        let mut device_vector = Vec::with_capacity(8);

        let enumeration = ffi::hid_enumerate(0, 0);
        {
            let mut current_device = enumeration;

            while !current_device.is_null() {
                device_vector.push(conv_hid_device_info(current_device)?);
                current_device = (*current_device).next;
            }
        }

        if !enumeration.is_null() {
            ffi::hid_free_enumeration(enumeration);
        }

        Ok(device_vector)
    }

    /// Returns vec of objects containing information about connected devices
    ///
    /// Deprecated. Use `HidApi::device_list()` instead.
    #[deprecated]
    pub fn devices(&self) -> &Vec<HidDeviceInfo> {
        &self.devices
    }

    /// Returns iterator containing information about attached HID devices.
    pub fn device_list(&self) -> impl Iterator<Item = &DeviceInfo> {
        self.device_list.iter()
    }

    /// Open a HID device using a Vendor ID (VID) and Product ID (PID).
    ///
    /// When multiple devices with the same vid and pid are available, then the
    /// first one found in the internal device list will be used. There are however
    /// no guarantees, which device this will be.
    pub fn open(&self, vid: u16, pid: u16) -> HidResult<HidDevice> {
        let device = unsafe { ffi::hid_open(vid, pid, std::ptr::null()) };

        if device.is_null() {
            match self.check_error() {
                Ok(err) => Err(err),
                Err(e) => Err(e),
            }
        } else {
            Ok(HidDevice {
                _hid_device: device,
                _lock: ManuallyDrop::new(self._lock.clone()),
            })
        }
    }

    /// Open a HID device using a Vendor ID (VID), Product ID (PID) and
    /// a serial number.
    pub fn open_serial(&self, vid: u16, pid: u16, sn: &str) -> HidResult<HidDevice> {
        let mut chars = sn.chars().map(|c| c as wchar_t).collect::<Vec<_>>();
        chars.push(0 as wchar_t);
        let device = unsafe { ffi::hid_open(vid, pid, chars.as_ptr()) };
        if device.is_null() {
            match self.check_error() {
                Ok(err) => Err(err),
                Err(e) => Err(e),
            }
        } else {
            Ok(HidDevice {
                _hid_device: device,
                _lock: ManuallyDrop::new(self._lock.clone()),
            })
        }
    }

    /// The path name be determined by inspecting the device list available with [HidApi::devices()](struct.HidApi.html#method.devices)
    ///
    /// Alternatively a platform-specific path name can be used (eg: /dev/hidraw0 on Linux).
    pub fn open_path(&self, device_path: &CStr) -> HidResult<HidDevice> {
        let device = unsafe { ffi::hid_open_path(device_path.as_ptr()) };

        if device.is_null() {
            match self.check_error() {
                Ok(err) => Err(err),
                Err(e) => Err(e),
            }
        } else {
            Ok(HidDevice {
                _hid_device: device,
                _lock: ManuallyDrop::new(self._lock.clone()),
            })
        }
    }

    /// Open a HID device using `libusb_wrap_sys_device`. Useful for Android.
    ///
    /// ### Arguments
    ///
    /// * `sys_dev`: Platform-specific file descriptor that can be recognised by libusb.
    /// * `interface_num`: USB interface number of the device to be used as HID interface. Pass -1
    /// to select first HID interface of the device.
    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn wrap_sys_device(&self, sys_dev: i32, interface_num: i32) -> HidResult<HidDevice> {
        let device = unsafe { ffi::hid_libusb_wrap_sys_device(sys_dev as _, interface_num) };

        if device.is_null() {
            match self.check_error() {
                Ok(err) => Err(err),
                Err(e) => Err(e),
            }
        } else {
            Ok(HidDevice {
                _hid_device: device,
                _lock: ManuallyDrop::new(self._lock.clone()),
            })
        }
    }

    /// Get the last non-device specific error, which happened in the underlying hidapi C library.
    /// To get the last device specific error, use [`HidDevice::check_error`].
    ///
    /// The `Ok()` variant of the result will contain a [HidError::HidApiError](enum.HidError.html).
    ///
    /// When `Err()` is returned, then acquiring the error string from the hidapi C
    /// library failed. The contained [HidError](enum.HidError.html) is the cause, why no error could
    /// be fetched.
    pub fn check_error(&self) -> HidResult<HidError> {
        Ok(HidError::HidApiError {
            message: unsafe {
                match wchar_to_string(ffi::hid_error(std::ptr::null_mut())) {
                    WcharString::String(s) => s,
                    _ => return Err(HidError::HidApiErrorEmpty),
                }
            },
        })
    }
}

/// Converts a pointer to a `*const wchar_t` to a WcharString.
unsafe fn wchar_to_string(wstr: *const wchar_t) -> WcharString {
    if wstr.is_null() {
        return WcharString::None;
    }

    let mut char_vector: Vec<char> = Vec::with_capacity(8);
    let mut raw_vector: Vec<wchar_t> = Vec::with_capacity(8);
    let mut index: isize = 0;
    let mut invalid_char = false;

    let o = |i| *wstr.offset(i);

    while o(index) != 0 {
        use std::char;

        raw_vector.push(*wstr.offset(index));

        if !invalid_char {
            if let Some(c) = char::from_u32(o(index) as u32) {
                char_vector.push(c);
            } else {
                invalid_char = true;
            }
        }

        index += 1;
    }

    if !invalid_char {
        WcharString::String(char_vector.into_iter().collect())
    } else {
        WcharString::Raw(raw_vector)
    }
}

/// Convert the CFFI `HidDeviceInfo` struct to a native `HidDeviceInfo` struct
unsafe fn conv_hid_device_info(src: *mut ffi::HidDeviceInfo) -> HidResult<DeviceInfo> {
    Ok(DeviceInfo {
        path: CStr::from_ptr((*src).path).to_owned(),
        vendor_id: (*src).vendor_id,
        product_id: (*src).product_id,
        serial_number: wchar_to_string((*src).serial_number),
        release_number: (*src).release_number,
        manufacturer_string: wchar_to_string((*src).manufacturer_string),
        product_string: wchar_to_string((*src).product_string),
        usage_page: (*src).usage_page,
        usage: (*src).usage,
        interface_number: (*src).interface_number,
    })
}

#[derive(Clone)]
enum WcharString {
    String(String),
    Raw(Vec<wchar_t>),
    None,
}

impl Into<Option<String>> for WcharString {
    fn into(self) -> Option<String> {
        match self {
            WcharString::String(s) => Some(s),
            _ => None,
        }
    }
}

/// Storage for device related information
///
/// Deprecated. Use `HidApi::device_list()` instead.
#[derive(Debug, Clone)]
#[deprecated]
pub struct HidDeviceInfo {
    pub path: CString,
    pub vendor_id: u16,
    pub product_id: u16,
    pub serial_number: Option<String>,
    pub release_number: u16,
    pub manufacturer_string: Option<String>,
    pub product_string: Option<String>,
    pub usage_page: u16,
    pub usage: u16,
    pub interface_number: i32,
}

/// Device information. Use accessors to extract information about Hid devices.
///
/// Note: Methods like `serial_number()` may return None, if the conversion to a
/// String failed internally. You can however access the raw hid representation of the
/// string by calling `serial_number_raw()`
#[derive(Clone)]
pub struct DeviceInfo {
    path: CString,
    vendor_id: u16,
    product_id: u16,
    serial_number: WcharString,
    release_number: u16,
    manufacturer_string: WcharString,
    product_string: WcharString,
    usage_page: u16,
    usage: u16,
    interface_number: i32,
}

impl DeviceInfo {
    pub fn path(&self) -> &CStr {
        &self.path
    }
    pub fn vendor_id(&self) -> u16 {
        self.vendor_id
    }
    pub fn product_id(&self) -> u16 {
        self.product_id
    }

    /// Try to call `serial_number_raw()`, if None is returned.
    pub fn serial_number(&self) -> Option<&str> {
        match self.serial_number {
            WcharString::String(ref s) => Some(s),
            _ => None,
        }
    }
    pub fn serial_number_raw(&self) -> Option<&[wchar_t]> {
        match self.serial_number {
            WcharString::Raw(ref s) => Some(s),
            _ => None,
        }
    }

    pub fn release_number(&self) -> u16 {
        self.release_number
    }

    /// Try to call `manufacturer_string_raw()`, if None is returned.
    pub fn manufacturer_string(&self) -> Option<&str> {
        match self.manufacturer_string {
            WcharString::String(ref s) => Some(s),
            _ => None,
        }
    }
    pub fn manufacturer_string_raw(&self) -> Option<&[wchar_t]> {
        match self.manufacturer_string {
            WcharString::Raw(ref s) => Some(s),
            _ => None,
        }
    }

    /// Try to call `product_string_raw()`, if None is returned.
    pub fn product_string(&self) -> Option<&str> {
        match self.product_string {
            WcharString::String(ref s) => Some(s),
            _ => None,
        }
    }
    pub fn product_string_raw(&self) -> Option<&[wchar_t]> {
        match self.product_string {
            WcharString::Raw(ref s) => Some(s),
            _ => None,
        }
    }

    pub fn usage_page(&self) -> u16 {
        self.usage_page
    }
    pub fn usage(&self) -> u16 {
        self.usage
    }
    pub fn interface_number(&self) -> i32 {
        self.interface_number
    }

    /// Use the information contained in `DeviceInfo` to open
    /// and return a handle to a [HidDevice](struct.HidDevice.html).
    ///
    /// By default the device path is used to open the device.
    /// When no path is available, then vid, pid and serial number are used.
    /// If both path and serial number are not available, then this function will
    /// fail with [HidError::OpenHidDeviceWithDeviceInfoError](enum.HidError.html#variant.OpenHidDeviceWithDeviceInfoError).
    ///
    /// Note, that opening a device could still be done using [HidApi::open()](struct.HidApi.html#method.open) directly.
    pub fn open_device(&self, hidapi: &HidApi) -> HidResult<HidDevice> {
        if self.path.as_bytes().len() != 0 {
            hidapi.open_path(self.path.as_c_str())
        } else if let Some(ref sn) = self.serial_number() {
            hidapi.open_serial(self.vendor_id, self.product_id, sn)
        } else {
            Err(HidError::OpenHidDeviceWithDeviceInfoError {
                device_info: Box::new(self.clone().into()),
            })
        }
    }
}

impl fmt::Debug for DeviceInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HidDeviceInfo")
            .field("vendor_id", &self.vendor_id)
            .field("product_id", &self.product_id)
            .finish()
    }
}

impl Into<HidDeviceInfo> for DeviceInfo {
    fn into(self) -> HidDeviceInfo {
        HidDeviceInfo {
            path: self.path,
            vendor_id: self.vendor_id,
            product_id: self.product_id,
            serial_number: match self.serial_number {
                WcharString::String(s) => Some(s),
                _ => None,
            },
            release_number: self.release_number,
            manufacturer_string: match self.manufacturer_string {
                WcharString::String(s) => Some(s),
                _ => None,
            },
            product_string: match self.product_string {
                WcharString::String(s) => Some(s),
                _ => None,
            },
            usage_page: self.usage_page,
            usage: self.usage,
            interface_number: self.interface_number,
        }
    }
}

impl HidDeviceInfo {
    /// Use the information contained in `HidDeviceInfo` to open
    /// and return a handle to a [HidDevice](struct.HidDevice.html).
    ///
    /// By default the device path is used to open the device.
    /// When no path is available, then vid, pid and serial number are used.
    /// If both path and serial number are not available, then this function will
    /// fail with [HidError::OpenHidDeviceWithDeviceInfoError](enum.HidError.html#variant.OpenHidDeviceWithDeviceInfoError).
    ///
    /// Note, that opening a device could still be done using [HidApi::open()](struct.HidApi.html#method.open) directly.
    pub fn open_device(&self, hidapi: &HidApi) -> HidResult<HidDevice> {
        if self.path.as_bytes().len() != 0 {
            hidapi.open_path(self.path.as_c_str())
        } else if let Some(ref sn) = self.serial_number {
            hidapi.open_serial(self.vendor_id, self.product_id, sn)
        } else {
            Err(HidError::OpenHidDeviceWithDeviceInfoError {
                device_info: Box::new(self.clone()),
            })
        }
    }
}

/// Object for accessing HID device
pub struct HidDevice {
    _hid_device: *mut ffi::HidDevice,
    /// Prevents this from outliving the api instance that created it
    _lock: ManuallyDrop<Arc<HidApiLock>>,
}

unsafe impl Send for HidDevice {}

impl Drop for HidDevice {
    fn drop(&mut self) {
        unsafe {
            ffi::hid_close(self._hid_device);
            ManuallyDrop::drop(&mut self._lock);
        };
    }
}

impl HidDevice {
    /// Check size returned by other methods, if it's equal to -1 check for
    /// error and return Error, otherwise return size as unsigned number
    fn check_size(&self, res: i32) -> HidResult<usize> {
        if res == -1 {
            match self.check_error() {
                Ok(err) => Err(err),
                Err(e) => Err(e),
            }
        } else {
            Ok(res as usize)
        }
    }

    /// Get the last error, which happened in the underlying hidapi C library.
    ///
    /// The `Ok()` variant of the result will contain a [HidError::HidApiError](enum.HidError.html).
    ///
    /// When `Err()` is returned, then acquiring the error string from the hidapi C
    /// library failed. The contained [HidError](enum.HidError.html) is the cause, why no error could
    /// be fetched.
    pub fn check_error(&self) -> HidResult<HidError> {
        Ok(HidError::HidApiError {
            message: unsafe {
                match wchar_to_string(ffi::hid_error(self._hid_device)) {
                    WcharString::String(s) => s,
                    _ => return Err(HidError::HidApiErrorEmpty),
                }
            },
        })
    }

    /// The first byte of `data` must contain the Report ID. For
    /// devices which only support a single report, this must be set
    /// to 0x0. The remaining bytes contain the report data. Since
    /// the Report ID is mandatory, calls to `write()` will always
    /// contain one more byte than the report contains. For example,
    /// if a hid report is 16 bytes long, 17 bytes must be passed to
    /// `write()`, the Report ID (or 0x0, for devices with a
    /// single report), followed by the report data (16 bytes). In
    /// this example, the length passed in would be 17.
    /// `write()` will send the data on the first OUT endpoint, if
    /// one exists. If it does not, it will send the data through
    /// the Control Endpoint (Endpoint 0).
    pub fn write(&self, data: &[u8]) -> HidResult<usize> {
        if data.len() == 0 {
            return Err(HidError::InvalidZeroSizeData);
        }
        let res = unsafe { ffi::hid_write(self._hid_device, data.as_ptr(), data.len() as size_t) };
        self.check_size(res)
    }

    /// Input reports are returned to the host through the 'INTERRUPT IN'
    /// endpoint. The first byte will contain the Report number if the device
    /// uses numbered reports.
    pub fn read(&self, buf: &mut [u8]) -> HidResult<usize> {
        let res = unsafe { ffi::hid_read(self._hid_device, buf.as_mut_ptr(), buf.len() as size_t) };
        self.check_size(res)
    }

    /// Input reports are returned to the host through the 'INTERRUPT IN'
    /// endpoint. The first byte will contain the Report number if the device
    /// uses numbered reports. Timeout measured in milliseconds, set -1 for
    /// blocking wait.
    pub fn read_timeout(&self, buf: &mut [u8], timeout: i32) -> HidResult<usize> {
        let res = unsafe {
            ffi::hid_read_timeout(
                self._hid_device,
                buf.as_mut_ptr(),
                buf.len() as size_t,
                timeout,
            )
        };
        self.check_size(res)
    }

    /// Send a Feature report to the device.
    /// Feature reports are sent over the Control endpoint as a
    /// Set_Report transfer.  The first byte of `data` must contain the
    /// 'Report ID'. For devices which only support a single report, this must
    /// be set to 0x0. The remaining bytes contain the report data. Since the
    /// 'Report ID' is mandatory, calls to `send_feature_report()` will always
    /// contain one more byte than the report contains. For example, if a hid
    /// report is 16 bytes long, 17 bytes must be passed to
    /// `send_feature_report()`: 'the Report ID' (or 0x0, for devices which
    /// do not use numbered reports), followed by the report data (16 bytes).
    /// In this example, the length passed in would be 17.
    pub fn send_feature_report(&self, data: &[u8]) -> HidResult<()> {
        if data.len() == 0 {
            return Err(HidError::InvalidZeroSizeData);
        }
        let res = unsafe {
            ffi::hid_send_feature_report(self._hid_device, data.as_ptr(), data.len() as size_t)
        };
        let res = self.check_size(res)?;
        if res != data.len() {
            Err(HidError::IncompleteSendError {
                sent: res,
                all: data.len(),
            })
        } else {
            Ok(())
        }
    }

    /// Set the first byte of `buf` to the 'Report ID' of the report to be read.
    /// Upon return, the first byte will still contain the Report ID, and the
    /// report data will start in buf[1].
    pub fn get_feature_report(&self, buf: &mut [u8]) -> HidResult<usize> {
        let res = unsafe {
            ffi::hid_get_feature_report(self._hid_device, buf.as_mut_ptr(), buf.len() as size_t)
        };
        self.check_size(res)
    }

    /// Set the device handle to be in blocking or in non-blocking mode. In
    /// non-blocking mode calls to `read()` will return immediately with an empty
    /// slice if there is no data to be read. In blocking mode, `read()` will
    /// wait (block) until there is data to read before returning.
    /// Modes can be changed at any time.
    pub fn set_blocking_mode(&self, blocking: bool) -> HidResult<()> {
        let res = unsafe {
            ffi::hid_set_nonblocking(self._hid_device, if blocking { 0i32 } else { 1i32 })
        };
        if res == -1 {
            Err(HidError::SetBlockingModeError {
                mode: match blocking {
                    true => "blocking",
                    false => "not blocking",
                },
            })
        } else {
            Ok(())
        }
    }

    /// Get The Manufacturer String from a HID device.
    pub fn get_manufacturer_string(&self) -> HidResult<Option<String>> {
        let mut buf = [0 as wchar_t; STRING_BUF_LEN];
        let res = unsafe {
            ffi::hid_get_manufacturer_string(
                self._hid_device,
                buf.as_mut_ptr(),
                STRING_BUF_LEN as size_t,
            )
        };
        let res = self.check_size(res)?;
        unsafe { Ok(wchar_to_string(buf[..res].as_ptr()).into()) }
    }

    /// Get The Manufacturer String from a HID device.
    pub fn get_product_string(&self) -> HidResult<Option<String>> {
        let mut buf = [0 as wchar_t; STRING_BUF_LEN];
        let res = unsafe {
            ffi::hid_get_product_string(
                self._hid_device,
                buf.as_mut_ptr(),
                STRING_BUF_LEN as size_t,
            )
        };
        let res = self.check_size(res)?;
        unsafe { Ok(wchar_to_string(buf[..res].as_ptr()).into()) }
    }

    /// Get The Serial Number String from a HID device.
    pub fn get_serial_number_string(&self) -> HidResult<Option<String>> {
        let mut buf = [0 as wchar_t; STRING_BUF_LEN];
        let res = unsafe {
            ffi::hid_get_serial_number_string(
                self._hid_device,
                buf.as_mut_ptr(),
                STRING_BUF_LEN as size_t,
            )
        };
        let res = self.check_size(res)?;
        unsafe { Ok(wchar_to_string(buf[..res].as_ptr()).into()) }
    }

    /// Get a string from a HID device, based on its string index.
    pub fn get_indexed_string(&self, index: i32) -> HidResult<Option<String>> {
        let mut buf = [0 as wchar_t; STRING_BUF_LEN];
        let res = unsafe {
            ffi::hid_get_indexed_string(
                self._hid_device,
                index as c_int,
                buf.as_mut_ptr(),
                STRING_BUF_LEN,
            )
        };
        let res = self.check_size(res)?;
        unsafe { Ok(wchar_to_string(buf[..res].as_ptr()).into()) }
    }
}
