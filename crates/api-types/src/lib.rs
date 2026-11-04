//! Wire types. Used by `server` (axum) and by the egui apps, so both sides
//! share exactly the same structs.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignupRequest {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgotRequest {
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetRequest {
    pub token: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyRequest {
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserDto {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub email_verified: bool,
}

/// Returned by login / signup. `token` is for native clients that cannot use
/// the cookie jar (send it as `Authorization: Bearer <token>`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub user: UserDto,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
}

/// One page as stored on the server. `data` is an `nw_core::Page` serialised
/// as JSON; the server never needs to understand it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPage {
    pub id: Uuid,
    pub updated: u64,
    pub deleted: bool,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushRequest {
    pub pages: Vec<SyncPage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullResponse {
    pub pages: Vec<SyncPage>,
    pub server_time: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDto {
    pub id: Uuid,
    pub created_ms: u64,
    pub user_agent: String,
    pub current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadInfo {
    pub version: String,
    pub file: String,
    pub sha256: String,
    pub size_bytes: u64,
}
