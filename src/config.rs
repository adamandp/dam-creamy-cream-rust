use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub database_url: String,
    pub redis_url: String,
    pub access_jwt_secret: String,
    pub refresh_jwt_secret: String,
    pub public_base_url: String,
    pub frontend_origins: Vec<String>,
    pub minio_endpoint: String,
    pub minio_access_key: String,
    pub minio_secret_key: String,
    pub minio_bucket: String,
    pub minio_region: String,
    pub production: bool,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        dotenvy::dotenv().ok();
        fn required(key: &str) -> Result<String, String> {
            env::var(key).map_err(|_| format!("missing required environment variable {key}"))
        }
        let port = env::var("PORT")
            .unwrap_or_else(|_| "3000".to_owned())
            .parse::<u16>()
            .map_err(|_| "PORT must be a valid port".to_owned())?;
        let access_jwt_secret = required("ACCESS_JWT_SECRET")?;
        let refresh_jwt_secret = required("REFRESH_JWT_SECRET")?;
        if access_jwt_secret.len() < 32 || refresh_jwt_secret.len() < 32 {
            return Err("JWT secrets must be at least 32 bytes".to_owned());
        }
        if access_jwt_secret == refresh_jwt_secret {
            return Err("ACCESS_JWT_SECRET and REFRESH_JWT_SECRET must differ".to_owned());
        }
        let production = env::var("NODE_ENV").is_ok_and(|v| v == "production");
        let public_base_url =
            env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| format!("http://localhost:{port}"));
        if !public_base_url.starts_with("http://") && !public_base_url.starts_with("https://") {
            return Err("PUBLIC_BASE_URL must be an HTTP(S) URL".to_owned());
        }
        let minio_endpoint = required("MINIO_ENDPOINT")?;
        if !minio_endpoint.starts_with("http://") && !minio_endpoint.starts_with("https://") {
            return Err("MINIO_ENDPOINT must include http:// or https://".to_owned());
        }
        let frontend_origins = env::var("FRONTEND_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>();
        if production && frontend_origins.is_empty() {
            return Err("FRONTEND_ORIGINS is required in production".to_owned());
        }
        if production && !minio_endpoint.starts_with("https://") {
            return Err("MINIO_ENDPOINT must use HTTPS for browser uploads in production".to_owned());
        }
        Ok(Self {
            port,
            database_url: required("DATABASE_URL")?,
            redis_url: required("REDIS_URL")?,
            access_jwt_secret,
            refresh_jwt_secret,
            public_base_url: public_base_url.trim_end_matches('/').into(),
            frontend_origins,
            minio_endpoint,
            minio_access_key: required("MINIO_ACCESS_KEY")?,
            minio_secret_key: required("MINIO_SECRET_KEY")?,
            minio_bucket: env::var("MINIO_BUCKET").unwrap_or_else(|_| "creamy-cream".into()),
            minio_region: env::var("MINIO_REGION").unwrap_or_else(|_| "us-east-1".into()),
            production,
        })
    }
}
