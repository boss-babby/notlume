use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use nw_api_types::ApiError;

#[derive(Debug)]
pub enum AppError {
    BadRequest(String),
    Unauthorized,
    Forbidden(&'static str),
    Conflict(&'static str),
    TooManyRequests,
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".to_owned()),
            Self::Forbidden(m) => (StatusCode::FORBIDDEN, m.to_owned()),
            Self::Conflict(m) => (StatusCode::CONFLICT, m.to_owned()),
            Self::TooManyRequests => (StatusCode::TOO_MANY_REQUESTS, "too many requests, slow down".to_owned()),
            Self::Internal(m) => {
                tracing::error!("internal error: {m}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_owned())
            }
        };
        (status, Json(ApiError { error: msg })).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(e.to_string())
    }
}
