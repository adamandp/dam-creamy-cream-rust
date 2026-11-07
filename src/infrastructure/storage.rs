use std::{io::Cursor, time::Duration};

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

    pub async fn verify_image(&self, key: &str) -> AppResult<String> {
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
        let mime = object.content_type().unwrap_or_default();
        if !matches!(mime, "image/jpeg" | "image/png" | "image/webp") {
            return Err(AppError::BadRequest(
                "Only JPEG, PNG or WebP images are supported".into(),
            ));
        }
        let etag = object
            .e_tag()
            .ok_or_else(|| AppError::BadRequest("Image has no object ETag".into()))?
            .to_owned();
        // Fully download and decode the verified object. Its HEAD size is
        // capped at 2 MiB, and image decoding has an independent pixel/memory
        // budget to reject decompression bombs and truncated file signatures.
        let uploaded = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .if_match(etag.as_str())
            .send()
            .await
            .map_err(|_| AppError::BadRequest("Image changed during verification".into()))?;
        let data = uploaded
            .body
            .collect()
            .await
            .map_err(|_| AppError::BadRequest("Could not read image content".into()))?
            .into_bytes();
        if data.len() as i64 != size || !valid_image(mime, data.as_ref()) {
            return Err(AppError::BadRequest(
                "Uploaded image is corrupt, oversized, or does not match its image type".into(),
            ));
        }
        Ok(etag)
    }

    pub async fn copy_verified_image(
        &self,
        source: &str,
        destination: &str,
        etag: &str,
    ) -> AppResult<()> {
        self.client
            .copy_object()
            .bucket(&self.bucket)
            .key(destination)
            .copy_source(format!("{}/{}", self.bucket, source))
            .copy_source_if_match(etag)
            .send()
            .await
            .map_err(|e| {
                AppError::BadRequest(format!("Image changed or could not be finalized: {e}"))
            })?;
        Ok(())
    }

    pub async fn delete_image(&self, key: &str) -> AppResult<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("MinIO delete failed: {e}")))?;
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

fn valid_image_signature(mime: &str, data: &[u8]) -> bool {
    match mime {
        "image/jpeg" => data.starts_with(&[0xff, 0xd8, 0xff]),
        "image/png" => {
            data.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10])
                && data.get(12..16) == Some(b"IHDR".as_slice())
        }
        "image/webp" => {
            let codec = data.get(12..16);
            data.starts_with(b"RIFF")
                && data.get(8..12) == Some(b"WEBP".as_slice())
                && (codec == Some(b"VP8 ".as_slice())
                    || codec == Some(b"VP8L".as_slice())
                    || codec == Some(b"VP8X".as_slice()))
        }
        _ => false,
    }
}

fn valid_image(mime: &str, data: &[u8]) -> bool {
    if !valid_image_signature(mime, data) || data.len() > 2_097_152 {
        return false;
    }
    let format = match mime {
        "image/jpeg" => image::ImageFormat::Jpeg,
        "image/png" => image::ImageFormat::Png,
        "image/webp" => image::ImageFormat::WebP,
        _ => return false,
    };
    let mut reader = image::ImageReader::with_format(Cursor::new(data), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().is_ok()
}

#[cfg(test)]
mod signature_tests {
    use std::io::Cursor;

    use super::{valid_image, valid_image_signature};

    #[test]
    fn truncated_magic_bytes_are_not_valid_decodable_images() {
        assert!(!valid_image(
            "image/png",
            b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR"
        ));
        assert!(!valid_image("image/jpeg", &[0xff, 0xd8, 0xff, 1]));
        assert!(!valid_image("image/webp", b"RIFF\x00\x00\x00\x00WEBPVP8 "));
    }

    #[test]
    fn real_png_round_trip_is_accepted() {
        let mut result = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1)
            .write_to(&mut result, image::ImageFormat::Png)
            .unwrap();
        assert!(valid_image("image/png", result.get_ref()));
        assert!(!valid_image("image/jpeg", result.get_ref()));
    }

    #[test]
    fn image_magic_bytes_must_match_declared_mime() {
        assert!(valid_image_signature("image/jpeg", &[0xff, 0xd8, 0xff, 1]));
        assert!(valid_image_signature(
            "image/png",
            b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR"
        ));
        assert!(valid_image_signature(
            "image/webp",
            b"RIFF\x00\x00\x00\x00WEBPVP8 "
        ));
        assert!(!valid_image_signature("image/png", b"not a real image"));
        assert!(!valid_image_signature("image/jpeg", b"\x89PNG\r\n\x1a\n"));
        assert!(!valid_image_signature(
            "image/webp",
            b"RIFF\x00\x00\x00\x00WEBPFAKE"
        ));
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

    #[tokio::test]
    async fn minio_verifies_images_and_finalizes_immutable_objects() {
        let Ok(endpoint) = std::env::var("TEST_MINIO_ENDPOINT") else {
            return;
        };
        let config = Config {
            port: 3000,
            database_url: String::new(),
            redis_url: String::new(),
            access_jwt_secret: String::new(),
            refresh_jwt_secret: String::new(),
            public_base_url: String::new(),
            frontend_origins: Vec::new(),
            minio_endpoint: endpoint,
            minio_access_key: "test-access-key".into(),
            minio_secret_key: "test-secret-key".into(),
            minio_bucket: "creamy-cream-ci".into(),
            minio_region: "us-east-1".into(),
            production: false,
        };
        let storage = Storage::connect(&config).await.unwrap();
        let user = uuid::Uuid::new_v4();
        let asset = uuid::Uuid::new_v4();
        let pending = format!("pending/users/{user}/{asset}");
        let permanent = format!("users/{user}/{asset}");
        let mut sample = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1)
            .write_to(&mut sample, image::ImageFormat::Png)
            .unwrap();
        let sample = sample.into_inner();
        storage
            .client
            .put_object()
            .bucket(&storage.bucket)
            .key(&pending)
            .content_type("image/png")
            .body(aws_sdk_s3::primitives::ByteStream::from(sample.clone()))
            .send()
            .await
            .unwrap();
        let etag = storage.verify_image(&pending).await.unwrap();
        storage
            .copy_verified_image(&pending, &permanent, &etag)
            .await
            .unwrap();
        storage.delete_image(&pending).await.unwrap();
        assert!(
            storage
                .client
                .head_object()
                .bucket(&storage.bucket)
                .key(&pending)
                .send()
                .await
                .is_err()
        );
        let actual = storage
            .client
            .get_object()
            .bucket(&storage.bucket)
            .key(&permanent)
            .send()
            .await
            .unwrap()
            .body
            .collect()
            .await
            .unwrap()
            .into_bytes();
        assert_eq!(actual.as_ref(), sample.as_slice());

        let invalid = format!("pending/users/{user}/{}", uuid::Uuid::new_v4());
        storage
            .client
            .put_object()
            .bucket(&storage.bucket)
            .key(&invalid)
            .content_type("image/png")
            .body(aws_sdk_s3::primitives::ByteStream::from(
                b"not-an-image".to_vec(),
            ))
            .send()
            .await
            .unwrap();
        assert!(matches!(
            storage.verify_image(&invalid).await,
            Err(AppError::BadRequest(_))
        ));
        storage.delete_image(&invalid).await.unwrap();
        storage.delete_image(&permanent).await.unwrap();
    }
}
