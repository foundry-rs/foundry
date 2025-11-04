//! Beacon API types and utilities for Anvil
//!
//! This module provides types and utilities for implementing Beacon API endpoints
//! in Anvil, allowing testing of blob-based transactions with standard beacon chain APIs.

pub mod error;
pub mod response;

pub use error::BeaconError;
pub use response::BeaconResponse;
