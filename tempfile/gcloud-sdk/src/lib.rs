//! # Google Cloud SDK for Rust
//!
//! Library provides all available APIs generated based:
//! - on proto interfaces for gRPC;
//! - OpenAPI spec for REST APIs not available as gRPC;
//!
//! The library also provides and easy-to-use client API for both gRPC and REST,
//! that supports Google Authentication natively.
//!
//! ## gRPC example
//! ```ignore
//!
//!     let firestore_client: GoogleApi<FirestoreClient<GoogleAuthMiddleware>> =
//!        GoogleApi::from_function(
//!            FirestoreClient::new,
//!            "https://firestore.googleapis.com",
//!            // cloud resource prefix: used only for some of the APIs (such as Firestore)
//!            Some(cloud_resource_prefix.clone()),
//!        )
//!        .await?;
//!
//!     let response = firestore_client
//!         .get()
//!         .list_documents(tonic::Request::new(ListDocumentsRequest {
//!             parent: format!("{}/documents", cloud_resource_prefix),
//!             ..Default::default()
//!         }))
//!         .await?;
//!
//! ```
//! ## REST example
//! ```ignore
//!
//! let google_rest_client = gcloud_sdk::GoogleRestApi::new().await?;
//!
//! let response = gcloud_sdk::google_rest_apis::storage::buckets_api::storage_buckets_list(
//!     &google_rest_client.create_google_storage_config().await?,
//!     gcloud_sdk::google_rest_apis::storage::buckets_api::StoragePeriodBucketsPeriodListParams {
//!         project: google_project_id,
//!         ..Default::default()
//!     }
//! ).await?;
//!
//! ```
//!
//! Complete examples available on [github](https://github.com/abdolence/gcloud-sdk-rs/tree/master/src/examples).
//!

mod apis;
pub use apis::*;

pub mod error;
mod token_source;
pub use token_source::auth_token_generator::GoogleAuthTokenGenerator;
pub use token_source::metadata::Metadata as GceMetadataClient;
pub use token_source::{ExternalJwtFunctionSource, Token, TokenSourceType};

mod api_client;
pub use api_client::*;

mod middleware;

pub mod proto_ext;

#[cfg(feature = "rest")]
mod rest_apis;

#[cfg(feature = "rest")]
pub use rest_apis::*;

pub const GCLOUD_SDK_USER_AGENT: &str = concat!("gcloud-sdk-rs/v", env!("CARGO_PKG_VERSION"));

// Re-exports
pub use prost;
pub use prost_types;
pub use secret_vault_value::SecretValue;
pub use tonic;
