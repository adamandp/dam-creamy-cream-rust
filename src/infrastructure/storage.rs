use std::time::Duration;

use aws_credential_types::Credentials;
use aws_sdk_s3::{
    Client,
    config::{Builder, Region},
    presigning::PresigningConfig,
};

use crate::{
    config::Config,
    error::{AppError, AppResult},
};

#[derive(Clone)]
pub struct Storage {
    client: Client,
    bucket: String,
}

impl Storage {
    fn new(cfg: &Config) -> Self {
        let credentials = Credentials::new(
            &cfg.minio_access_key,
            &cfg.minio_secret_key,
            None,
            None,
            "minio",
        );
        let sdk_config = Builder::new()
            .region(Region::new(cfg.minio_region.clone()))
            .credentials_provider(credentials)
            .endpoint_url(&cfg.minio_endpoint)
            .force_path_style(true)
            .build();
        Self {
            client: Client::from_conf(sdk_config),
            bucket: cfg.minio_bucket.clone(),
        }
    }

    #[cfg(test)]
    pub(crate) fn offline(cfg: &Config) -> Self {
        Self::new(cfg)
    }

    pub async fn connect(cfg: &Config) -> Result<Self, String> {
        let storage = Self::new(cfg);
        if storage
            .client
            .head_bucket()
            .bucket(&storage.bucket)
            .send()
            .await
            .is_err()
        {
            storage
                .client
                .create_bucket()
                .bucket(&storage.bucket)
                .send()
                .await
                .map_err(|e| format!("could not access or create MinIO bucket: {e}"))?;
        }
        Ok(storage)
    }

    pub async fn presign_put(&self, key: &str, mime: &str, size: i64) -> AppResult<String> {
        let config = PresigningConfig::expires_in(Duration::from_secs(10 * 60))
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let request = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(mime)
            .content_length(size)
            .presigned(config)
            .await
            .map_err(|e| AppError::Internal(format!("MinIO presign PUT failed: {e}")))?;
        Ok(request.uri().to_string())
    }

    pub async fn verify_image(&self, key: &str) -> AppResult<()> {
        let object = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|_| AppError::BadRequest("Image was not uploaded to MinIO".into()))?;
        let size = object.content_length().unwrap_or_default();
        if !(1..=2_097_152).contains(&size) {
            return Err(AppError::BadRequest(
                "Image size must be at most 2 MiB".into(),
            ));
        }
        if !matches!(
            object.content_type(),
            Some("image/jpeg" | "image/png" | "image/webp")
        ) {
            return Err(AppError::BadRequest(
                "Only JPEG, PNG or WebP images are supported".into(),
            ));
        }
        Ok(())
    }

    pub async fn presign_get(&self, key: &str) -> AppResult<String> {
        let config = PresigningConfig::expires_in(Duration::from_secs(5 * 60))
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let request = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(config)
            .await
            .map_err(|e| AppError::Internal(format!("MinIO presign GET failed: {e}")))?;
        Ok(request.uri().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn presigned_put_signs_upload_headers() {
        let config = Config {
            port: 3000,
            database_url: String::new(),
            redis_url: String::new(),
            access_jwt_secret: String::new(),
            refresh_jwt_secret: String::new(),
            public_base_url: String::new(),
            frontend_origins: Vec::new(),
            minio_endpoint: "http://localhost:9000".into(),
            minio_access_key: "test-access-key".into(),
            minio_secret_key: "test-secret-key".into(),
            minio_bucket: "creamy-cream".into(),
            minio_region: "us-east-1".into(),
            production: false,
        };
        let signed = Storage::offline(&config)
            .presign_put("users/example/photo", "image/webp", 12345)
            .await
            .unwrap();
        assert!(signed.contains("X-Amz-Signature="));
        assert!(signed.contains("X-Amz-SignedHeaders="));
        assert!(signed.contains("content-length"));
        assert!(signed.contains("content-type"));
    }
}
