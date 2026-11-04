//! Browser glue: localStorage persistence, downloads, file import, boot.

use nw_core::{Page, Settings, Uuid, Workspace};
use nw_ui::app::{AppOptions, Imports, Platform};
use nw_ui::{NotionApp, Persistence};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

const PREFIX: &str = "nw:";

fn window() -> Option<web_sys::Window> {
    web_sys::window()
}

fn storage() -> Option<web_sys::Storage> {
    window()?.local_storage().ok().flatten()
}

/// Synchronous persistence in `localStorage` (one key per page). The server
/// sync (see `nw_ui::sync_client`) is the durable, cross-device copy.
/// An IndexedDB backend is a drop-in replacement once `Persistence` gains an
/// async flavour; see docs/adr/0001-architecture.md.
pub struct LocalStorageStore;

impl LocalStorageStore {
    fn get(key: &str) -> Option<String> {
        storage()?.get_item(&format!("{PREFIX}{key}")).ok().flatten()
    }

    fn set(key: &str, value: &str) {
        if let Some(s) = storage() {
            if s.set_item(&format!("{PREFIX}{key}"), value).is_err() {
                log::warn!("localStorage quota exceeded while saving {key}");
            }
        }
    }

    fn index() -> Vec<String> {
        Self::get("index").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
}

impl Persistence for LocalStorageStore {
    fn load(&mut self) -> Option<(Workspace, Settings)> {
        let ids = Self::index();
        let settings: Option<Settings> = Self::get("settings").and_then(|s| serde_json::from_str(&s).ok());
        if ids.is_empty() && settings.is_none() {
            return None;
        }
        let mut ws = Workspace::default();
        for id in ids {
            if let Some(p) = Self::get(&format!("page:{id}")).and_then(|j| serde_json::from_str::<Page>(&j).ok()) {
                ws.pages.push(p);
            }
        }
        if let Some(v) = Self::get("versions").and_then(|j| serde_json::from_str(&j).ok()) {
            ws.versions = v;
        }
        Some((ws, settings.unwrap_or_default()))
    }

    fn save_page(&mut self, page: &Page) {
        if let Ok(json) = serde_json::to_string(page) {
            Self::set(&format!("page:{}", page.id), &json);
        }
        let mut ids = Self::index();
        let id = page.id.to_string();
        if !ids.contains(&id) {
            ids.push(id);
            if let Ok(j) = serde_json::to_string(&ids) {
                Self::set("index", &j);
            }
        }
    }

    fn remove_page(&mut self, id: Uuid) {
        let id = id.to_string();
        if let Some(s) = storage() {
            let _ = s.remove_item(&format!("{PREFIX}page:{id}"));
        }
        let ids: Vec<String> = Self::index().into_iter().filter(|i| *i != id).collect();
        if let Ok(j) = serde_json::to_string(&ids) {
            Self::set("index", &j);
        }
    }

    fn save_settings(&mut self, settings: &Settings) {
        if let Ok(j) = serde_json::to_string(settings) {
            Self::set("settings", &j);
        }
    }

    fn save_versions(&mut self, ws: &Workspace) {
        // Keep only the 5 newest per page in the browser to stay under quota.
        let mut trimmed = ws.versions.clone();
        trimmed.sort_by_key(|v| std::cmp::Reverse(v.ts));
        let mut seen: std::collections::HashMap<Uuid, usize> = std::collections::HashMap::new();
        trimmed.retain(|v| {
            let n = seen.entry(v.page).or_insert(0);
            *n += 1;
            *n <= 5
        });
        if let Ok(j) = serde_json::to_string(&trimmed) {
            Self::set("versions", &j);
        }
    }
}

struct WebPlatform;

impl Platform for WebPlatform {
    fn save_file(&self, filename: &str, mime: &str, data: &[u8]) {
        let result = (|| -> Result<(), JsValue> {
            let doc = window().and_then(|w| w.document()).ok_or("no document")?;
            let bytes = js_sys::Uint8Array::from(data);
            let parts = js_sys::Array::of1(&bytes);
            let opts = web_sys::BlobPropertyBag::new();
            opts.set_type(mime);
            let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts)?;
            let url = web_sys::Url::create_object_url_with_blob(&blob)?;
            let a: web_sys::HtmlAnchorElement = doc.create_element("a")?.dyn_into()?;
            a.set_href(&url);
            a.set_download(filename);
            a.click();
            web_sys::Url::revoke_object_url(&url)?;
            Ok(())
        })();
        if let Err(e) = result {
            log::error!("download failed: {e:?}");
        }
    }

    fn request_import(&self, ctx: &egui::Context, inbox: Imports) {
        let Some(doc) = window().and_then(|w| w.document()) else { return };
        let Ok(el) = doc.create_element("input") else { return };
        let Ok(input) = el.dyn_into::<web_sys::HtmlInputElement>() else { return };
        input.set_type("file");
        input.set_accept(".md,.markdown,.txt,.json");
        let ctx = ctx.clone();
        let input_in_closure = input.clone();
        let on_change = Closure::once_into_js(move || {
            let Some(file) = input_in_closure.files().and_then(|f| f.get(0)) else { return };
            let name = file.name();
            let Ok(reader) = web_sys::FileReader::new() else { return };
            let reader_in_closure = reader.clone();
            let on_load = Closure::once_into_js(move || {
                if let Some(text) = reader_in_closure.result().ok().and_then(|r| r.as_string()) {
                    if let Ok(mut g) = inbox.lock() {
                        g.push((name, text));
                    }
                    ctx.request_repaint();
                }
            });
            reader.set_onload(Some(on_load.unchecked_ref()));
            let _ = reader.read_as_text(&file);
        });
        input.set_onchange(Some(on_change.unchecked_ref()));
        input.click();
    }
}

pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let Some(win) = window() else { return };
        let Some(doc) = win.document() else { return };
        let canvas = doc
            .get_element_by_id("nw_canvas")
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok());
        let Some(canvas) = canvas else {
            log::error!("canvas #nw_canvas not found");
            return;
        };
        let origin = win.location().origin().unwrap_or_default();
        let reduced = win
            .match_media("(prefers-reduced-motion: reduce)")
            .ok()
            .flatten()
            .is_some_and(|m| m.matches());
        let opts = AppOptions {
            platform: Box::new(WebPlatform),
            is_web: true,
            download_url: format!("{origin}/download"),
            api_base: Some(origin.clone()),
            website_url: origin,
            prefers_reduced_motion: reduced,
        };
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |cc| Ok(Box::new(NotionApp::new(cc, Box::new(LocalStorageStore), opts)))),
            )
            .await;
        if let Some(splash) = doc.get_element_by_id("loading") {
            match result {
                Ok(()) => splash.remove(),
                Err(e) => {
                    splash.set_inner_html("<p>The app failed to start. See the browser console for details.</p>");
                    log::error!("failed to start eframe: {e:?}");
                }
            }
        }
    });
}
