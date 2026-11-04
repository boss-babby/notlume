#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod store;

use nw_ui::app::{AppOptions, Imports, Platform};
use nw_ui::NotionApp;
use store::SqliteStore;

struct NativePlatform;

impl Platform for NativePlatform {
    fn save_file(&self, filename: &str, _mime: &str, data: &[u8]) {
        if let Some(path) = rfd::FileDialog::new().set_file_name(filename).save_file() {
            if let Err(e) = std::fs::write(&path, data) {
                log::error!("could not write {}: {e}", path.display());
            }
        }
    }

    fn request_import(&self, ctx: &egui::Context, inbox: Imports) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Markdown / JSON", &["md", "markdown", "txt", "json"])
            .pick_file()
        else {
            return;
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let name = path.file_name().map_or_else(|| "Imported".into(), |n| n.to_string_lossy().into_owned());
                if let Ok(mut g) = inbox.lock() {
                    g.push((name, text));
                }
                ctx.request_repaint();
            }
            Err(e) => log::error!("could not read {}: {e}", path.display()),
        }
    }
}

fn main() -> eframe::Result {
    env_logger::init();
    let db_path = std::env::var_os("NW_DB")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| dirs::data_dir().unwrap_or_else(|| ".".into()).join("Notes").join("workspace.db"));
    let store = SqliteStore::open(&db_path).unwrap_or_else(|e| {
        log::error!("cannot open {}: {e}; falling back to memory", db_path.display());
        SqliteStore::open_in_memory().expect("in-memory sqlite")
    });

    let website = std::env::var("NW_WEBSITE_URL").unwrap_or_else(|_| "http://localhost:3000".to_owned());
    let opts = AppOptions {
        platform: Box::new(NativePlatform),
        is_web: false,
        download_url: format!("{website}/download"),
        api_base: Some(website.clone()),
        website_url: website,
        prefers_reduced_motion: false,
    };

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Notes")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([720.0, 480.0]),
        renderer: eframe::Renderer::Wgpu,
        vsync: true,
        ..Default::default()
    };
    eframe::run_native(
        "Notes",
        native_options,
        Box::new(move |cc| Ok(Box::new(NotionApp::new(cc, Box::new(store), opts)))),
    )
}
