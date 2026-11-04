//! Sign-up, login, sessions, email verification and password reset.
//!
//! * Passwords: argon2id (library defaults, PHC string), hashed on the blocking pool.
//! * Sessions: 256-bit random token; only its SHA-256 is stored. Delivered as an
//!   `HttpOnly; SameSite=Lax` cookie (browsers) or `Authorization: Bearer` (native app).
//! * Emails: delivery is abstracted by [`send_mail`]; the default implementation
//!   logs the message so development works without SMTP. Plug `lettre` in there.

use crate::error::AppError;
use crate::{security, AppState};
use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::{FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use nw_api_types::{
    AuthResponse, ForgotRequest, LoginRequest, ResetRequest, SessionDto, SignupRequest, UserDto, VerifyRequest,
};
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

pub const COOKIE: &str = "nw_session";
const SESSION_MS: i64 = 30 * 24 * 3600 * 1000;
const VERIFY_MS: i64 = 48 * 3600 * 1000;
const RESET_MS: i64 = 3600 * 1000;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn new_token() -> String {
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

pub fn hash_token(t: &str) -> String {
    hex::encode(Sha256::digest(t.as_bytes()))
}

/// Password policy: >= 10 chars, at least one letter and one digit, not the email.
pub fn validate_password(pw: &str, email: &str) -> Result<(), String> {
    if pw.chars().count() < 10 {
        return Err("password must be at least 10 characters".into());
    }
    if pw.len() > 200 {
        return Err("password is too long".into());
    }
    if !pw.chars().any(char::is_alphabetic) || !pw.chars().any(|c| c.is_ascii_digit()) {
        return Err("password must contain letters and digits".into());
    }
    if pw.eq_ignore_ascii_case(email) {
        return Err("password must not be your email".into());
    }
    Ok(())
}

pub fn validate_email(e: &str) -> bool {
    e.len() <= 254 && e.split_once('@').is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.starts_with('.') && !e.contains(' '))
}

async fn hash_password(pw: String) -> Result<String, AppError> {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(pw.as_bytes(), &SaltString::generate(&mut OsRng))
            .map(|h| h.to_string())
            .map_err(|e| AppError::Internal(e.to_string()))
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
}

async fn verify_password(pw: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash).is_ok_and(|h| Argon2::default().verify_password(pw.as_bytes(), &h).is_ok())
    })
    .await
    .unwrap_or(false)
}

/// Development mailer: logs the message. Replace with SMTP in production.
pub fn send_mail(to: &str, subject: &str, body: &str) {
    tracing::info!(target: "mail", "to={to} subject={subject:?}\n{body}");
}

pub fn token_from_headers(h: &HeaderMap) -> Option<String> {
    if let Some(b) = h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")) {
        return Some(b.trim().to_owned());
    }
    h.get(header::COOKIE)
        .and_then(|v| v.to_str().ok())?
        .split(';')
        .filter_map(|c| c.trim().split_once('='))
        .find(|(k, _)| *k == COOKIE)
        .map(|(_, v)| v.to_owned())
}

fn cookie_header(token: &str, max_age_s: i64, secure: bool) -> HeaderValue {
    let s = format!(
        "{COOKIE}={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age_s}{}",
        if secure { "; Secure" } else { "" }
    );
    HeaderValue::from_str(&s).unwrap_or_else(|_| HeaderValue::from_static(""))
}

pub struct AuthUser {
    pub user_id: Uuid,
    pub session_id: Uuid,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let token = token_from_headers(&parts.headers).ok_or(AppError::Unauthorized)?;
        let row = sqlx::query("SELECT id, user_id FROM sessions WHERE token_hash = $1 AND expires_ms > $2")
            .bind(hash_token(&token))
            .bind(now_ms())
            .fetch_optional(&state.db)
            .await?
            .ok_or(AppError::Unauthorized)?;
        Ok(Self {
            session_id: row.get("id"),
            user_id: row.get("user_id"),
        })
    }
}

async fn create_session(s: &AppState, user_id: Uuid, headers: &HeaderMap) -> Result<String, AppError> {
    let token = new_token();
    let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("").chars().take(200).collect::<String>();
    sqlx::query("INSERT INTO sessions (id, user_id, token_hash, created_ms, expires_ms, user_agent) VALUES ($1,$2,$3,$4,$5,$6)")
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(hash_token(&token))
        .bind(now_ms())
        .bind(now_ms() + SESSION_MS)
        .bind(ua)
        .execute(&s.db)
        .await?;
    Ok(token)
}

async fn issue_email_token(s: &AppState, user_id: Uuid, kind: &str, ttl: i64) -> Result<String, AppError> {
    let token = new_token();
    sqlx::query("INSERT INTO email_tokens (token_hash, user_id, kind, expires_ms) VALUES ($1,$2,$3,$4)")
        .bind(hash_token(&token))
        .bind(user_id)
        .bind(kind)
        .bind(now_ms() + ttl)
        .execute(&s.db)
        .await?;
    Ok(token)
}

fn user_dto(row: &sqlx::postgres::PgRow) -> UserDto {
    UserDto {
        id: row.get("id"),
        email: row.get("email"),
        name: row.get("name"),
        email_verified: row.get("email_verified"),
    }
}

async fn signup(State(s): State<AppState>, headers: HeaderMap, Json(req): Json<SignupRequest>) -> Result<impl IntoResponse, AppError> {
    let email = req.email.trim().to_lowercase();
    if !validate_email(&email) {
        return Err(AppError::BadRequest("invalid email address".into()));
    }
    validate_password(&req.password, &email).map_err(AppError::BadRequest)?;
    let hash = hash_password(req.password).await?;
    let id = Uuid::new_v4();
    let name: String = req.name.trim().chars().take(80).collect();
    let res = sqlx::query("INSERT INTO users (id, email, name, password_hash, created_ms) VALUES ($1,$2,$3,$4,$5)")
        .bind(id)
        .bind(&email)
        .bind(&name)
        .bind(hash)
        .bind(now_ms())
        .execute(&s.db)
        .await;
    match res {
        Ok(_) => {}
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => return Err(AppError::Conflict("email already registered")),
        Err(e) => return Err(e.into()),
    }
    let vt = issue_email_token(&s, id, "verify", VERIFY_MS).await?;
    send_mail(&email, "Verify your email", &format!("{}/verify-email?token={vt}", s.cfg.public_url));
    let token = create_session(&s, id, &headers).await?;
    let mut h = HeaderMap::new();
    h.insert(header::SET_COOKIE, cookie_header(&token, SESSION_MS / 1000, s.cfg.cookie_secure));
    let user = UserDto { id, email, name, email_verified: false };
    Ok((StatusCode::CREATED, h, Json(AuthResponse { user, token })))
}

async fn login(State(s): State<AppState>, headers: HeaderMap, Json(req): Json<LoginRequest>) -> Result<impl IntoResponse, AppError> {
    let email = req.email.trim().to_lowercase();
    let row = sqlx::query("SELECT id, email, name, email_verified, password_hash FROM users WHERE email = $1")
        .bind(&email)
        .fetch_optional(&s.db)
        .await?;
    let Some(row) = row else {
        // Spend comparable time so response timing does not reveal which emails exist.
        let _ = hash_password(req.password).await;
        return Err(AppError::Unauthorized);
    };
    if !verify_password(req.password, row.get("password_hash")).await {
        return Err(AppError::Unauthorized);
    }
    let user = user_dto(&row);
    let token = create_session(&s, user.id, &headers).await?;
    let mut h = HeaderMap::new();
    h.insert(header::SET_COOKIE, cookie_header(&token, SESSION_MS / 1000, s.cfg.cookie_secure));
    Ok((StatusCode::OK, h, Json(AuthResponse { user, token })))
}

async fn logout(State(s): State<AppState>, headers: HeaderMap) -> Result<impl IntoResponse, AppError> {
    if let Some(t) = token_from_headers(&headers) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1").bind(hash_token(&t)).execute(&s.db).await?;
    }
    let mut h = HeaderMap::new();
    h.insert(header::SET_COOKIE, cookie_header("", 0, s.cfg.cookie_secure));
    Ok((StatusCode::OK, h, Json(serde_json::json!({ "ok": true }))))
}

async fn me(State(s): State<AppState>, user: AuthUser) -> Result<Json<UserDto>, AppError> {
    let row = sqlx::query("SELECT id, email, name, email_verified FROM users WHERE id = $1")
        .bind(user.user_id)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::Unauthorized)?;
    Ok(Json(user_dto(&row)))
}

async fn verify(State(s): State<AppState>, Json(req): Json<VerifyRequest>) -> Result<Json<serde_json::Value>, AppError> {
    let row = sqlx::query("DELETE FROM email_tokens WHERE token_hash = $1 AND kind = 'verify' AND expires_ms > $2 RETURNING user_id")
        .bind(hash_token(&req.token))
        .bind(now_ms())
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::BadRequest("invalid or expired token".into()))?;
    let uid: Uuid = row.get("user_id");
    sqlx::query("UPDATE users SET email_verified = TRUE WHERE id = $1").bind(uid).execute(&s.db).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn forgot(State(s): State<AppState>, Json(req): Json<ForgotRequest>) -> Result<Json<serde_json::Value>, AppError> {
    let email = req.email.trim().to_lowercase();
    if let Some(row) = sqlx::query("SELECT id FROM users WHERE email = $1").bind(&email).fetch_optional(&s.db).await? {
        let uid: Uuid = row.get("id");
        let t = issue_email_token(&s, uid, "reset", RESET_MS).await?;
        send_mail(&email, "Reset your password", &format!("{}/reset-password?token={t}", s.cfg.public_url));
    }
    // Same answer whether or not the account exists.
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn reset(State(s): State<AppState>, Json(req): Json<ResetRequest>) -> Result<Json<serde_json::Value>, AppError> {
    let row = sqlx::query(
        "DELETE FROM email_tokens WHERE token_hash = $1 AND kind = 'reset' AND expires_ms > $2 RETURNING user_id",
    )
    .bind(hash_token(&req.token))
    .bind(now_ms())
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::BadRequest("invalid or expired token".into()))?;
    let uid: Uuid = row.get("user_id");
    let email: String = sqlx::query("SELECT email FROM users WHERE id = $1").bind(uid).fetch_one(&s.db).await?.get("email");
    validate_password(&req.password, &email).map_err(AppError::BadRequest)?;
    let hash = hash_password(req.password).await?;
    sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2").bind(hash).bind(uid).execute(&s.db).await?;
    // A reset signs out every device.
    sqlx::query("DELETE FROM sessions WHERE user_id = $1").bind(uid).execute(&s.db).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn list_sessions(State(s): State<AppState>, user: AuthUser) -> Result<Json<Vec<SessionDto>>, AppError> {
    let rows = sqlx::query("SELECT id, created_ms, user_agent FROM sessions WHERE user_id = $1 ORDER BY created_ms DESC")
        .bind(user.user_id)
        .fetch_all(&s.db)
        .await?;
    Ok(Json(
        rows.iter()
            .map(|r| {
                let id: Uuid = r.get("id");
                SessionDto {
                    id,
                    created_ms: r.get::<i64, _>("created_ms") as u64,
                    user_agent: r.get("user_agent"),
                    current: id == user.session_id,
                }
            })
            .collect(),
    ))
}

async fn revoke_session(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> Result<Json<serde_json::Value>, AppError> {
    sqlx::query("DELETE FROM sessions WHERE id = $1 AND user_id = $2").bind(id).bind(user.user_id).execute(&s.db).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/signup", post(signup))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/forgot", post(forgot))
        .route("/reset", post(reset))
        .route("/verify", post(verify))
        .route("/me", get(me))
        .route("/sessions", get(list_sessions))
        .route("/sessions/{id}", delete(revoke_session))
        // Strict per-IP limit on every auth endpoint (credential stuffing, email bombing).
        .layer(middleware::from_fn_with_state(state, security::rate_limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_policy() {
        assert!(validate_password("short1", "a@b.co").is_err());
        assert!(validate_password("onlyletterslong", "a@b.co").is_err());
        assert!(validate_password("1234567890123", "a@b.co").is_err());
        assert!(validate_password("correct-horse-9", "a@b.co").is_ok());
    }

    #[test]
    fn email_validation() {
        assert!(validate_email("a@b.co"));
        assert!(!validate_email("nope"));
        assert!(!validate_email("a@b"));
        assert!(!validate_email("a b@c.de"));
    }

    #[test]
    fn token_parsing() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_static("a=1; nw_session=abc; b=2"));
        assert_eq!(token_from_headers(&h).as_deref(), Some("abc"));
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer xyz"));
        assert_eq!(token_from_headers(&h).as_deref(), Some("xyz"));
    }

    #[test]
    fn tokens_are_hashed_and_unique() {
        let (a, b) = (new_token(), new_token());
        assert_ne!(a, b);
        assert_eq!(hash_token(&a).len(), 64);
        assert_ne!(hash_token(&a), a);
    }
}
