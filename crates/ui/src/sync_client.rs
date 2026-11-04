//! Non-blocking HTTP client for login and page sync. `ehttp` works on native
//! (threads) and on wasm (fetch), so this file is shared by both.
//!
//! Results arrive on a shared inbox that the app drains once per frame.

use nw_api_types::{ApiError, AuthResponse, LoginRequest, PullResponse, PushRequest, SyncPage};
use nw_core::Page;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct Inbox {
    pub login: Option<Result<AuthResponse, String>>,
    pub pulled: Option<Result<PullResponse, String>>,
    pub pushed: Option<Result<(), String>>,
}

#[derive(Clone)]
pub struct SyncClient {
    pub base_url: String,
    pub token: Option<String>,
    pub inbox: Arc<Mutex<Inbox>>,
}

fn error_text(resp: &ehttp::Response) -> String {
    serde_json::from_slice::<ApiError>(&resp.bytes)
        .map(|e| e.error)
        .unwrap_or_else(|_| format!("HTTP {}", resp.status))
}

impl SyncClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            token: None,
            inbox: Arc::default(),
        }
    }

    fn request(&self, mut req: ehttp::Request) -> ehttp::Request {
        req.headers.insert("Content-Type", "application/json");
        req.headers.insert("X-Requested-With", "nw-app");
        if let Some(t) = &self.token {
            req.headers.insert("Authorization", format!("Bearer {t}"));
        }
        req
    }

    pub fn login(&self, email: &str, password: &str, ctx: &egui::Context) {
        let body = serde_json::to_vec(&LoginRequest {
            email: email.to_owned(),
            password: password.to_owned(),
        })
        .unwrap_or_default();
        let req = self.request(ehttp::Request::post(format!("{}/api/auth/login", self.base_url), body));
        let (inbox, ctx) = (self.inbox.clone(), ctx.clone());
        ehttp::fetch(req, move |res| {
            let out = match res {
                Ok(r) if r.ok => serde_json::from_slice::<AuthResponse>(&r.bytes).map_err(|e| e.to_string()),
                Ok(r) => Err(error_text(&r)),
                Err(e) => Err(e),
            };
            if let Ok(mut i) = inbox.lock() {
                i.login = Some(out);
            }
            ctx.request_repaint();
        });
    }

    pub fn pull(&self, ctx: &egui::Context) {
        let req = self.request(ehttp::Request::get(format!("{}/api/sync/pull", self.base_url)));
        let (inbox, ctx) = (self.inbox.clone(), ctx.clone());
        ehttp::fetch(req, move |res| {
            let out = match res {
                Ok(r) if r.ok => serde_json::from_slice::<PullResponse>(&r.bytes).map_err(|e| e.to_string()),
                Ok(r) => Err(error_text(&r)),
                Err(e) => Err(e),
            };
            if let Ok(mut i) = inbox.lock() {
                i.pulled = Some(out);
            }
            ctx.request_repaint();
        });
    }

    pub fn push(&self, pages: &[&Page], deleted: &[nw_core::Uuid], ctx: &egui::Context) {
        let mut items: Vec<SyncPage> = pages
            .iter()
            .filter_map(|p| {
                Some(SyncPage {
                    id: p.id,
                    updated: p.updated,
                    deleted: false,
                    data: serde_json::to_value(p).ok()?,
                })
            })
            .collect();
        items.extend(deleted.iter().map(|id| SyncPage {
            id: *id,
            updated: nw_core::time::now_ms(),
            deleted: true,
            data: serde_json::Value::Null,
        }));
        let body = serde_json::to_vec(&PushRequest { pages: items }).unwrap_or_default();
        let req = self.request(ehttp::Request::post(format!("{}/api/sync/push", self.base_url), body));
        let (inbox, ctx) = (self.inbox.clone(), ctx.clone());
        ehttp::fetch(req, move |res| {
            let out = match res {
                Ok(r) if r.ok => Ok(()),
                Ok(r) => Err(error_text(&r)),
                Err(e) => Err(e),
            };
            if let Ok(mut i) = inbox.lock() {
                i.pushed = Some(out);
            }
            ctx.request_repaint();
        });
    }
}
