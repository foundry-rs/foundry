// **************************************************************************
// Copyright (c) 2018 Roland Ruckerbauer All Rights Reserved.
//
// This file is part of hidapi-rs, based on hidapi-rs by Osspial
// **************************************************************************

use super::HidDeviceInfo;
use libc::wchar_t;
use std::error::Error;
use std::fmt::{Display, Formatter, Result};

#[derive(Debug)]
pub enum HidError {
    HidApiError {
        message: String,
    },
    #[deprecated]
    HidApiErrorEmptyWithCause {
        cause: Box<dyn Error + Send + Sync>,
    },
    HidApiErrorEmpty,
    FromWideCharError {
        wide_char: wchar_t,
    },
    InitializationError,
    #[deprecated]
    OpenHidDeviceError,
    InvalidZeroSizeData,
    IncompleteSendError {
        sent: usize,
        all: usize,
    },
    SetBlockingModeError {
        mode: &'static str,
    },
    OpenHidDeviceWithDeviceInfoError {
        device_info: Box<HidDeviceInfo>,
    },
}

impl Display for HidError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            HidError::HidApiError { message } => write!(f, "hidapi error: {}", message),
            HidError::HidApiErrorEmptyWithCause { cause } => write!(
                f,
                "hidapi error: (could not get error message), caused by: {}",
                cause
            ),
            HidError::HidApiErrorEmpty => write!(f, "hidapi error: (could not get error message)"),
            HidError::FromWideCharError { wide_char } => {
                write!(f, "failed converting {:#X} to rust char", wide_char)
            }
            HidError::InitializationError => {
                write!(f, "Failed to initialize hidapi (maybe initialized before?)")
            }
            HidError::OpenHidDeviceError => write!(f, "Failed opening hid device"),
            HidError::InvalidZeroSizeData => write!(f, "Invalid data: size can not be 0"),
            HidError::IncompleteSendError { sent, all } => write!(
                f,
                "Failed to send all data: only sent {} out of {} bytes",
                sent, all
            ),
            HidError::SetBlockingModeError { mode } => {
                write!(f, "Can not set blocking mode to '{}'", mode)
            }
            HidError::OpenHidDeviceWithDeviceInfoError { device_info } => {
                write!(f, "Can not open hid device with: {:?}", *device_info)
            }
        }
    }
}

impl Error for HidError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            HidError::HidApiErrorEmptyWithCause { cause } => Some(cause.as_ref()),
            _ => None,
        }
    }
}
