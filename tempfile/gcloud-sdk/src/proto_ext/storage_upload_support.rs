use crate::{create_hyper_uri_with_params, GoogleRestApi};
use reqwest::Body;

pub type BoxStreamWithSync<'a, T> =
    std::pin::Pin<Box<dyn futures::Stream<Item = T> + Send + 'a + Sync>>;

impl GoogleRestApi {
    #[deprecated(
        since = "0.19.4",
        note = "please use `gcloud_sdk::google_rest_apis::storage_v1::objects_api::storage_objects_insert_ext_stream` instead"
    )]
    pub async fn google_storage_v1_upload_stream_bytes<T: ToString>(
        &self,
        bucket_name: String,
        filename: String,
        content_type: T,
        bytes_stream: BoxStreamWithSync<
            'static,
            std::result::Result<bytes::Bytes, Box<(dyn std::error::Error + Send + Sync + 'static)>>,
        >,
    ) -> crate::error::Result<crate::google_rest_apis::storage_v1::object::Object> {
        let upload_url = format!(
            "https://storage.googleapis.com/upload/storage/v1/b/{}/o",
            bucket_name
        );

        let response = self
            .with_google_token(
                self.client.post(
                    &create_hyper_uri_with_params(
                        upload_url.as_str(),
                        &vec![
                            ("name", Some(filename).as_ref()),
                            ("uploadType", Some("media".into()).as_ref()),
                        ],
                    )
                    .to_string(),
                ),
            )
            .await?
            .header(reqwest::header::CONTENT_TYPE, content_type.to_string())
            .body(Body::wrap_stream(bytes_stream))
            .send()
            .await?;

        let json_result = response
            .json::<crate::google_rest_apis::storage_v1::object::Object>()
            .await?;

        Ok(json_result)
    }
}
