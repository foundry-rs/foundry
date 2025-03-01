use crate::error::ErrorKind;
use hyper::http::uri::PathAndQuery;
use std::env;
use tracing::*;

#[derive(Debug)]
pub struct GceMetadataClient {
    availability: GceMetadataClientAvailability,
}

#[derive(Debug)]
enum GceMetadataClientAvailability {
    Available(reqwest::Client, String),
    Unavailable,
    NotVerified,
}

const GCE_METADATA_HOST_ENV: &str = "GCE_METADATA_HOST";
const GCE_METADATA_IP: &str = "169.254.169.254";

impl GceMetadataClient {
    pub fn new() -> Self {
        Self {
            availability: GceMetadataClientAvailability::NotVerified,
        }
    }

    pub async fn init(&mut self) -> bool {
        match self.availability {
            GceMetadataClientAvailability::Available(_, _) => return true,
            GceMetadataClientAvailability::Unavailable => return false,
            GceMetadataClientAvailability::NotVerified => {}
        }
        debug!("GCE metadata server client init");
        let mut default_headers = reqwest::header::HeaderMap::new();
        default_headers.append(
            "Metadata-Flavor",
            "Google".parse().expect("Metadata-Flavor header is valid"),
        );
        default_headers.append(
            reqwest::header::USER_AGENT,
            crate::GCLOUD_SDK_USER_AGENT
                .parse()
                .expect("User agent header is valid"),
        );
        let http_client = reqwest::Client::builder()
            .default_headers(default_headers)
            .timeout(std::time::Duration::from_secs(5))
            .tcp_keepalive(std::time::Duration::from_secs(60))
            .build();

        match http_client {
            Ok(client) => {
                let metadata_host_name = env::var(GCE_METADATA_HOST_ENV)
                    .ok()
                    .unwrap_or("metadata.google.internal".to_string());
                debug!("Metadata server host: {}", &metadata_host_name);
                let resolved = match tokio::net::lookup_host((metadata_host_name.clone(), 80)).await
                {
                    Ok(mut addrs) => {
                        if addrs.next().is_none() {
                            debug!("Metadata server address is not available through DNS");
                            self.availability = GceMetadataClientAvailability::Unavailable;
                            false
                        } else {
                            debug!("Metadata server address is detected through DNS");
                            self.availability = GceMetadataClientAvailability::Available(
                                client.clone(),
                                metadata_host_name,
                            );
                            true
                        }
                    }
                    Err(err) => {
                        debug!("Resolving metadata server address failed with: {}", err);
                        self.availability = GceMetadataClientAvailability::Unavailable;
                        false
                    }
                };

                if !resolved {
                    // Last resort, try to use IP address with HTTP call
                    match client
                        .get(&format!("http://{}/", GCE_METADATA_IP))
                        .send()
                        .await
                    {
                        Ok(response) => {
                            if response.status().is_success() {
                                debug!(
                                    "Metadata server is available through direct IP: {}",
                                    response.status()
                                );
                                self.availability = GceMetadataClientAvailability::Available(
                                    client,
                                    GCE_METADATA_IP.to_string(),
                                );
                                true
                            } else {
                                debug!("Metadata server address HTTP verification through direct IP failed with: {}", response.status());
                                self.availability = GceMetadataClientAvailability::Unavailable;
                                false
                            }
                        }
                        Err(err) => {
                            debug!(
                                "Metadata server address is not available through direct IP: {}",
                                err
                            );
                            self.availability = GceMetadataClientAvailability::Unavailable;
                            false
                        }
                    }
                } else {
                    resolved
                }
            }
            Err(e) => {
                error!("Error creating HTTP client: {}", e);
                self.availability = GceMetadataClientAvailability::Unavailable;
                false
            }
        }
    }

    pub fn is_available(&self) -> bool {
        match self.availability {
            GceMetadataClientAvailability::Available(_, _) => true,
            GceMetadataClientAvailability::Unavailable
            | GceMetadataClientAvailability::NotVerified => false,
        }
    }

    pub async fn get(&self, path_and_query: PathAndQuery) -> crate::error::Result<String> {
        match self.availability {
            GceMetadataClientAvailability::Available(ref client, ref metadata_server_host) => {
                let url = format!("http://{}{}", metadata_server_host, path_and_query.as_str());

                let response = client.get(&url).send().await?;

                let status = response.status();
                let body = response.text().await?;

                if status.is_success() {
                    Ok(body)
                } else {
                    Err(ErrorKind::Metadata(format!(
                        "Error retrieving data from metadata server: {}({}). URL: {}",
                        status, body, url
                    ))
                    .into())
                }
            }
            GceMetadataClientAvailability::Unavailable => Err(ErrorKind::Metadata(format!(
                "Error retrieving data from metadata server: {}",
                "Metadata server not available"
            ))
            .into()),
            GceMetadataClientAvailability::NotVerified => Err(ErrorKind::Metadata(format!(
                "Error retrieving data from metadata server: {}",
                "Metadata server client requires initialization"
            ))
            .into()),
        }
    }
}
