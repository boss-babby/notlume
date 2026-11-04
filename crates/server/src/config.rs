#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub bind: String,
    /// Public origin of the site, e.g. `https://notes.example.com`.
    pub public_url: String,
    pub allowed_origins: Vec<String>,
    pub cookie_secure: bool,
    /// Directory with the `trunk build` output, served at `/app`.
    pub web_dist: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        let public_url = std::env::var("PUBLIC_URL").unwrap_or_else(|_| "http://localhost:3000".into());
        let mut allowed_origins = vec![public_url.trim_end_matches('/').to_owned()];
        if let Ok(extra) = std::env::var("ALLOWED_ORIGINS") {
            allowed_origins.extend(extra.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()));
        }
        Self {
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5432/app_db".into()),
            bind: std::env::var("BIND").unwrap_or_else(|_| "0.0.0.0:8000".into()),
            cookie_secure: public_url.starts_with("https://"),
            public_url,
            allowed_origins,
            web_dist: std::env::var("WEB_DIST").ok().filter(|s| !s.is_empty()),
        }
    }

    pub fn for_tests(database_url: String) -> Self {
        Self {
            database_url,
            bind: "127.0.0.1:0".into(),
            public_url: "http://localhost:3000".into(),
            allowed_origins: vec!["http://localhost:3000".into()],
            cookie_secure: false,
            web_dist: None,
        }
    }
}
