//! Beacon API types and utilities for Anvil
//!
//! This module provides types and utilities for implementing Beacon API endpoints
//! in Anvil, allowing testing of blob-based transactions with standard beacon chain APIs.

pub mod data;
pub mod error;
pub mod response;

pub use data::GenesisDetails;
pub use error::BeaconError;
pub use response::BeaconResponse;
