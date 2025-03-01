#[cfg(any(feature = "google-rest-bigquery-v2"))]
pub mod bigquery_v2;

#[cfg(any(feature = "google-rest-compute-v1"))]
pub mod compute_v1;

#[cfg(any(feature = "google-rest-dns-v1"))]
pub mod dns_v1;

#[cfg(any(feature = "google-rest-sqladmin-v1"))]
pub mod sqladmin_v1;

#[cfg(any(feature = "google-rest-storage-v1"))]
pub mod storage_v1;
