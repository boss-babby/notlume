//! Rate limiting, CSRF guard and secure headers.

use crate::error::AppError;
use crate::AppState;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::Response;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tower_http::set_header::SetResponseHeaderLayer;

/// Fixed-window in-memory limiter keyed by client address.
/// Use a shared store (Redis) if you run more than one server instance.
pub struct RateLimiter {
    hits: Mutex<HashMap<String, (Instant, u32)>>,
    max: u32,
    window: Duration,
}

impl RateLimiter {
    pub fn new(max: u32, window: Duration) -> Self {
        Self {
            hits: Mutex::new(HashMap::new()),
            max,
            window,
        }
    }

    pub fn check(&self, key: &str) -> bool {
        let Ok(mut map) = self.hits.lock() else { return true };
        let now = Instant::now();
        if map.len() > 10_000 {
            map.retain(|_, (t, _)| now.duration_since(*t) < self.window);
        }
        let e = map.entry(key.to_owned()).or_insert((now, 0));
        if now.duration_since(e.0) >= self.window {
            *e = (now, 0);
        }
        e.1 += 1;
        e.1 <= self.max
    }
}

pub fn client_ip(h: &HeaderMap) -> String {
    h.get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map_or_else(|| "unknown".to_owned(), |s| s.trim().to_owned())
}

pub async fn rate_limit(State(s): State<AppState>, req: Request, next: Next) -> Result<Response, AppError> {
    if !s.limiter.check(&client_ip(req.headers())) {
        return Err(AppError::TooManyRequests);
    }
    Ok(next.run(req).await)
}

/// State-changing requests must carry `X-Requested-With` (which a plain HTML
/// form cannot set cross-site) and, if an `Origin` is sent, it must be allowed.
/// Together with `SameSite=Lax` cookies this blocks CSRF.
pub async fn csrf_guard(State(s): State<AppState>, req: Request, next: Next) -> Result<Response, AppError> {
    if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return Ok(next.run(req).await);
    }
    let h = req.headers();
    if !h.contains_key("x-requested-with") {
        return Err(AppError::Forbidden("missing X-Requested-With header"));
    }
    if let Some(origin) = h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        if !s.cfg.allowed_origins.iter().any(|o| o == origin) {
            return Err(AppError::Forbidden("origin not allowed"));
        }
    }
    Ok(next.run(req).await)
}

const CSP: &str = "default-src 'self'; \
script-src 'self' 'wasm-unsafe-eval' 'unsafe-inline'; \
style-src 'self' 'unsafe-inline'; \
img-src 'self' data: blob: https:; \
connect-src 'self' https:; \
font-src 'self' data:; \
worker-src 'self' blob:; \
frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

/// Header layers applied to every response. `wasm-unsafe-eval` is what lets
/// the browser compile the WebAssembly module under a strict CSP.
pub fn security_headers() -> Vec<SetResponseHeaderLayer<HeaderValue>> {
    let pairs: [(&'static str, &'static str); 6] = [
        ("strict-transport-security", "max-age=63072000; includeSubDomains; preload"),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "strict-origin-when-cross-origin"),
        ("permissions-policy", "camera=(), microphone=(), geolocation=()"),
        ("content-security-policy", CSP),
    ];
    pairs
        .into_iter()
        .map(|(k, v)| SetResponseHeaderLayer::overriding(HeaderName::from_static(k), HeaderValue::from_static(v)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_blocks_after_max() {
        let l = RateLimiter::new(3, Duration::from_secs(60));
        assert!(l.check("a") && l.check("a") && l.check("a"));
        assert!(!l.check("a"));
        assert!(l.check("b"));
    }
}
