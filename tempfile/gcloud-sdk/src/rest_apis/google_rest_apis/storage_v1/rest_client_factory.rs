impl crate::GoogleRestApi {
    pub async fn create_google_storage_v1_config(
        &self,
    ) -> crate::error::Result<crate::google_rest_apis::storage_v1::apis::configuration::Configuration>
    {
        let token = self.token_generator.create_token().await?;
        Ok(
            crate::google_rest_apis::storage_v1::apis::configuration::Configuration {
                client: self.client.clone(),
                user_agent: Some(crate::GCLOUD_SDK_USER_AGENT.to_string()),
                oauth_access_token: Some(token.token.as_sensitive_str().to_string()),
                ..Default::default()
            },
        )
    }
}
