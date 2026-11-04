//! Integration tests. They need a PostgreSQL database:
//! `TEST_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/app_db cargo test -p server`
//! Without the variable the tests are skipped.

use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use server::{build_router, connect, AppState, Config};
use tower::ServiceExt;

async fn app() -> Option<Router> {
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let cfg = Config::for_tests(url);
    let pool = connect(&cfg).await.expect("connect + migrate");
    Some(build_router(AppState::new(pool, cfg)))
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>, cookie: Option<&str>) -> (StatusCode, HeaderMap, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("x-requested-with", "test");
    if body.is_some() {
        req = req.header(header::CONTENT_TYPE, "application/json");
    }
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let req = req.body(Body::from(body.map(|b| b.to_string()).unwrap_or_default())).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let (parts, body) = res.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    (parts.status, parts.headers, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn signup_login_sync_roundtrip() {
    let Some(app) = app().await else {
        eprintln!("TEST_DATABASE_URL not set; skipping");
        return;
    };
    let email = format!("t{}@example.com", uuid::Uuid::new_v4().simple());
    let pw = "correct-horse-9";

    let (st, headers, body) = call(&app, "POST", "/api/auth/signup", Some(json!({ "email": email, "password": pw })), None).await;
    assert_eq!(st, StatusCode::CREATED, "{body}");
    let cookie = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap().split(';').next().unwrap().to_owned();
    assert!(headers.get(header::SET_COOKIE).unwrap().to_str().unwrap().contains("HttpOnly"));

    // duplicate and weak passwords are rejected
    let (st, _, _) = call(&app, "POST", "/api/auth/signup", Some(json!({ "email": email, "password": pw })), None).await;
    assert_eq!(st, StatusCode::CONFLICT);
    let (st, _, _) = call(&app, "POST", "/api/auth/signup", Some(json!({ "email": "x@example.com", "password": "weak" })), None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    let (st, _, me) = call(&app, "GET", "/api/auth/me", None, Some(&cookie)).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(me["email"], email);
    let (st, _, _) = call(&app, "GET", "/api/auth/me", None, None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);

    // CSRF: no X-Requested-With => forbidden
    let req = Request::builder().method("POST").uri("/api/auth/logout").body(Body::empty()).unwrap();
    assert_eq!(app.clone().oneshot(req).await.unwrap().status(), StatusCode::FORBIDDEN);

    // wrong password
    let (st, _, _) = call(&app, "POST", "/api/auth/login", Some(json!({ "email": email, "password": "nope-nope-1" })), None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);

    // sync: push then pull
    let id = uuid::Uuid::new_v4();
    let page = json!({ "pages": [{ "id": id, "updated": 100, "deleted": false, "data": { "title": "Hello" } }] });
    let (st, _, _) = call(&app, "POST", "/api/sync/push", Some(page), Some(&cookie)).await;
    assert_eq!(st, StatusCode::OK);
    // an older write must not overwrite a newer one
    let stale = json!({ "pages": [{ "id": id, "updated": 50, "deleted": false, "data": { "title": "Old" } }] });
    call(&app, "POST", "/api/sync/push", Some(stale), Some(&cookie)).await;
    let (st, _, pulled) = call(&app, "GET", "/api/sync/pull?since=0", None, Some(&cookie)).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(pulled["pages"][0]["data"]["title"], "Hello");

    // logout invalidates the session
    let (st, _, _) = call(&app, "POST", "/api/auth/logout", None, Some(&cookie)).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _, _) = call(&app, "GET", "/api/auth/me", None, Some(&cookie)).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}
