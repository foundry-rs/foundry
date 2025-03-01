use std::marker::PhantomData;
use std::time::Duration;

use crate::token_source::auth_token_generator::GoogleAuthTokenGenerator;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use tonic::transport::Channel;
use tower::ServiceBuilder;
use tracing::*;

use crate::middleware::{GoogleAuthMiddlewareLayer, GoogleAuthMiddlewareService};
use crate::token_source::*;

#[async_trait]
pub trait GoogleApiClientBuilder<C>
where
    C: Clone + Send,
{
    fn create_client(&self, channel: GoogleAuthMiddlewareService<Channel>) -> C;
}

#[derive(Clone)]
pub struct GoogleApiClient<B, C>
where
    B: GoogleApiClientBuilder<C>,
    C: Clone + Send,
{
    builder: B,
    service: GoogleAuthMiddlewareService<Channel>,
    _ph: PhantomData<C>,
}

impl<B, C> GoogleApiClient<B, C>
where
    B: GoogleApiClientBuilder<C>,
    C: Clone + Send,
{
    pub async fn with_token_source<S: AsRef<str>>(
        builder: B,
        google_api_url: S,
        cloud_resource_prefix: Option<String>,
        token_source_type: TokenSourceType,
        token_scopes: Vec<String>,
    ) -> crate::error::Result<Self> {
        debug!(
            "Creating a new Google API client for {}. Scopes: {:?}",
            google_api_url.as_ref(),
            token_scopes
        );

        let channel = GoogleEnvironment::init_google_services_channel(google_api_url).await?;

        let token_generator =
            GoogleAuthTokenGenerator::new(token_source_type, token_scopes).await?;

        let middleware = GoogleAuthMiddlewareLayer::new(token_generator, cloud_resource_prefix);

        let service: GoogleAuthMiddlewareService<Channel> =
            ServiceBuilder::new().layer(middleware).service(channel);

        Ok(Self {
            builder,
            service,
            _ph: PhantomData::default(),
        })
    }

    pub fn get(&self) -> C {
        self.builder.create_client(self.service.clone())
    }
}

#[derive(Clone)]
pub struct GoogleApiClientBuilderFunction<C>
where
    C: Clone + Send,
{
    f: fn(GoogleAuthMiddlewareService<Channel>) -> C,
}

impl<C> GoogleApiClientBuilder<C> for GoogleApiClientBuilderFunction<C>
where
    C: Clone + Send,
{
    fn create_client(&self, channel: GoogleAuthMiddlewareService<Channel>) -> C {
        (self.f)(channel)
    }
}

impl<C> GoogleApiClient<GoogleApiClientBuilderFunction<C>, C>
where
    C: Clone + Send,
{
    pub async fn from_function<S: AsRef<str>>(
        builder_fn: fn(GoogleAuthMiddlewareService<Channel>) -> C,
        google_api_url: S,
        cloud_resource_prefix_meta: Option<String>,
    ) -> crate::error::Result<Self> {
        Self::from_function_with_scopes(
            builder_fn,
            google_api_url,
            cloud_resource_prefix_meta,
            GCP_DEFAULT_SCOPES.clone(),
        )
        .await
    }

    pub async fn from_function_with_scopes<S: AsRef<str>>(
        builder_fn: fn(GoogleAuthMiddlewareService<Channel>) -> C,
        google_api_url: S,
        cloud_resource_prefix_meta: Option<String>,
        token_scopes: Vec<String>,
    ) -> crate::error::Result<Self> {
        Self::from_function_with_token_source(
            builder_fn,
            google_api_url,
            cloud_resource_prefix_meta,
            token_scopes,
            TokenSourceType::Default,
        )
        .await
    }

    pub async fn from_function_with_token_source<S: AsRef<str>>(
        builder_fn: fn(GoogleAuthMiddlewareService<Channel>) -> C,
        google_api_url: S,
        cloud_resource_prefix_meta: Option<String>,
        token_scopes: Vec<String>,
        token_source_type: TokenSourceType,
    ) -> crate::error::Result<Self> {
        let builder: GoogleApiClientBuilderFunction<C> =
            GoogleApiClientBuilderFunction { f: builder_fn };

        Self::with_token_source(
            builder,
            google_api_url,
            cloud_resource_prefix_meta,
            token_source_type,
            token_scopes,
        )
        .await
    }
}

pub type GoogleAuthMiddleware = GoogleAuthMiddlewareService<Channel>;
pub type GoogleApi<C> = GoogleApiClient<GoogleApiClientBuilderFunction<C>, C>;

pub struct GoogleEnvironment;

impl GoogleEnvironment {
    pub async fn detect_google_project_id() -> Option<String> {
        let for_env = std::env::var("GCP_PROJECT")
            .ok()
            .or_else(|| std::env::var("PROJECT_ID").ok())
            .or_else(|| std::env::var("GCP_PROJECT_ID").ok());
        if for_env.is_some() {
            debug!("Detected GCP Project ID using environment variables");
            for_env
        } else {
            let local_creds = match crate::token_source::from_env_var(&GCP_DEFAULT_SCOPES) {
                Ok(Some(creds)) => Some(creds),
                Ok(None) | Err(_) => crate::token_source::from_well_known_file(&GCP_DEFAULT_SCOPES)
                    .ok()
                    .flatten(),
            };

            let local_quota_project_id =
                local_creds.and_then(|creds| creds.quota_project_id().map(ToString::to_string));

            if local_quota_project_id.is_some() {
                debug!("Detected default project id from local defined in quota_project_id for the service account file.");
                local_quota_project_id
            } else {
                let mut metadata_server =
                    crate::token_source::metadata::Metadata::new(GCP_DEFAULT_SCOPES.clone());
                if metadata_server.init().await {
                    let metadata_result = metadata_server.detect_google_project_id().await;
                    if metadata_result.is_some() {
                        debug!("Detected GCP Project ID using GKE metadata server");
                        metadata_result
                    } else {
                        debug!("No GCP Project ID detected in this environment. Please specify it explicitly using environment variables: `PROJECT_ID`,`GCP_PROJECT_ID`, or `GCP_PROJECT`");
                        metadata_result
                    }
                } else {
                    debug!("No GCP Project ID detected in this environment. Please specify it explicitly using environment variables: `PROJECT_ID`,`GCP_PROJECT_ID`, or `GCP_PROJECT`");
                    None
                }
            }
        }
    }

    pub async fn init_google_services_channel<S: AsRef<str>>(
        api_url: S,
    ) -> Result<Channel, crate::error::Error> {
        let api_url_string = api_url.as_ref().to_string();
        let base_config = Channel::from_shared(api_url_string.clone())?
            .connect_timeout(Duration::from_secs(30))
            .tcp_keepalive(Some(Duration::from_secs(60)))
            .keep_alive_timeout(Duration::from_secs(60))
            .http2_keep_alive_interval(Duration::from_secs(60))
            .keep_alive_while_idle(true);

        let config = if !&api_url_string.contains("http://") {
            let domain_name = api_url_string.replace("https://", "");

            let tls_config = Self::init_tls_config(domain_name);
            base_config.tls_config(tls_config)?
        } else {
            base_config
        };

        Ok(config.connect().await?)
    }

    #[cfg(not(any(feature = "tls-roots", feature = "tls-webpki-roots")))]
    fn init_tls_config(domain_name: String) -> tonic::transport::ClientTlsConfig {
        tonic::transport::ClientTlsConfig::new()
            .ca_certificate(tonic::transport::Certificate::from_pem(
                crate::apis::CERTIFICATES,
            ))
            .domain_name(domain_name)
    }

    #[cfg(feature = "tls-roots")]
    fn init_tls_config(domain_name: String) -> tonic::transport::ClientTlsConfig {
        tonic::transport::ClientTlsConfig::new()
            .with_native_roots()
            .domain_name(domain_name)
    }

    #[cfg(feature = "tls-webpki-roots")]
    fn init_tls_config(domain_name: String) -> tonic::transport::ClientTlsConfig {
        tonic::transport::ClientTlsConfig::new()
            .with_webpki_roots()
            .domain_name(domain_name)
    }
}

pub static GCP_DEFAULT_SCOPES: Lazy<Vec<String>> =
    Lazy::new(|| vec!["https://www.googleapis.com/auth/cloud-platform".into()]);
