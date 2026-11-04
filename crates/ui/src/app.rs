//! Application shell: sidebar page tree, top bar, page view, command palette,
//! settings, version history, toasts, autosave and server sync.

use crate::anim;
use crate::db_view::{self, DbState};
use crate::editor::{self, EditorState};
use crate::fonts;
use crate::storage::Persistence;
use crate::sync_client::SyncClient;
use crate::theme::{self, Palette, ACCENTS};
use egui::collapsing_header::CollapsingState;
use egui::{pos2, vec2, Align, Align2, Color32, Context, FontId, Id, Key, Layout, Modifiers, Rect, RichText, Sense, Ui};
use nw_core::database::ymd_from_ms;
use nw_core::export;
use nw_core::markdown::blocks_to_markdown;
use nw_core::search::{fuzzy_score, SearchIndex};
use nw_core::sync::merge_pages;
use nw_core::time::now_ms;
use nw_core::{Block, Cover, History, Page, Settings, ThemeMode, Uuid, Workspace};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Files picked by the user for import: `(file name, text content)`.
pub type Imports = Arc<Mutex<Vec<(String, String)>>>;

/// Platform services that differ between native and web.
pub trait Platform {
    /// Save / download a file (native: save dialog, web: browser download).
    fn save_file(&self, filename: &str, mime: &str, data: &[u8]);
    /// Ask the user for a text file; push the result into `inbox` and
    /// call `ctx.request_repaint()` when it arrives.
    fn request_import(&self, ctx: &Context, inbox: Imports);
}

pub struct AppOptions {
    pub platform: Box<dyn Platform>,
    pub is_web: bool,
    /// Base URL of the website, e.g. `https://notes.example.com`.
    pub website_url: String,
    /// Where the "Download desktop app" link goes.
    pub download_url: String,
    /// Base URL of the sync API. `None` disables sync UI.
    pub api_base: Option<String>,
    /// Initial value of the OS "prefers reduced motion" setting.
    pub prefers_reduced_motion: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cmd {
    NewPage,
    NewDatabase,
    ToggleTheme,
    ToggleSidebar,
    ExportMd,
    ExportHtml,
    ExportJson,
    ExportPdf,
    ImportMd,
    DeletePage,
    ToggleFavorite,
    OpenSettings,
    OpenHistory,
    Undo,
    Redo,
    DownloadApp,
    OpenWebApp,
    OpenWebsite,
}

const CMDS: &[(Cmd, &str, &str)] = &[
    (Cmd::NewPage, "New page", "＋"),
    (Cmd::NewDatabase, "New database", "🗂"),
    (Cmd::ToggleTheme, "Toggle dark mode", "🌓"),
    (Cmd::ToggleSidebar, "Toggle sidebar", "◧"),
    (Cmd::ToggleFavorite, "Toggle favorite", "★"),
    (Cmd::ExportMd, "Export as Markdown", "⬇"),
    (Cmd::ExportHtml, "Export as HTML", "⬇"),
    (Cmd::ExportJson, "Export as JSON", "⬇"),
    (Cmd::ExportPdf, "Export as PDF", "⬇"),
    (Cmd::ImportMd, "Import Markdown / JSON file", "⬆"),
    (Cmd::OpenHistory, "Version history", "🕘"),
    (Cmd::Undo, "Undo", "↶"),
    (Cmd::Redo, "Redo", "↷"),
    (Cmd::OpenSettings, "Settings", "⚙"),
    (Cmd::DeletePage, "Delete page", "🗑"),
    (Cmd::DownloadApp, "Download desktop app", "⬇"),
    (Cmd::OpenWebApp, "Open web app", "🌐"),
    (Cmd::OpenWebsite, "Open website", "🏠"),
];

enum Item {
    Page(Uuid, String),
    Cmd(Cmd, &'static str, &'static str),
}

#[derive(Default)]
struct PaletteState {
    open: bool,
    query: String,
    sel: usize,
    just_opened: bool,
}

struct Toast {
    text: String,
    born: f64,
    ttl: f64,
}

#[derive(Clone, Copy)]
struct PageDrag(Uuid);

enum SideAction {
    Open(Uuid),
    AddChild(Uuid),
    Delete(Uuid),
    Favorite(Uuid),
    Move { id: Uuid, parent: Option<Uuid>, before: Option<Uuid> },
}

pub struct NotionApp {
    ws: Workspace,
    settings: Settings,
    store: Box<dyn Persistence>,
    opts: AppOptions,
    current: Option<Uuid>,
    history: History<Vec<Block>>,
    search: SearchIndex,
    editor: EditorState,
    dbs: DbState,
    palette: PaletteState,
    toasts: Vec<Toast>,
    dirty: HashSet<Uuid>,
    removed: Vec<Uuid>,
    last_edit: f64,
    last_version: f64,
    show_settings: bool,
    show_history: bool,
    pal_from: Palette,
    pal_to: Palette,
    pal_changed_at: f64,
    last_applied: Option<Palette>,
    imports: Imports,
    mentions_dirty: bool,
    page_opened_at: f64,
    // sync
    sync: Option<SyncClient>,
    sync_dirty: HashSet<Uuid>,
    sync_removed: Vec<Uuid>,
    sync_status: String,
    signed_in: bool,
    last_sync: f64,
    sync_busy: bool,
    login_email: String,
    login_password: String,
    cover_url: String,
}

fn fmt_ts(ms: u64) -> String {
    let (y, m, d) = ymd_from_ms(ms);
    let s = (ms / 1000) % 86_400;
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", s / 3600, (s % 3600) / 60)
}

impl NotionApp {
    pub fn new(cc: &eframe::CreationContext<'_>, mut store: Box<dyn Persistence>, opts: AppOptions) -> Self {
        fonts::install(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let (ws, mut settings) = store.load().unwrap_or_else(|| {
            let ws = Workspace::with_welcome();
            for p in &ws.pages {
                store.save_page(p);
            }
            (ws, Settings::default())
        });
        if opts.prefers_reduced_motion {
            settings.reduce_motion = true;
        }
        let mut search = SearchIndex::new();
        for p in &ws.pages {
            search.index_page(p);
        }
        let current = settings
            .last_page
            .filter(|id| ws.page(*id).is_some())
            .or_else(|| ws.pages.first().map(|p| p.id));
        let blocks = current.and_then(|id| ws.page(id)).map(|p| p.blocks.clone()).unwrap_or_default();
        let pal = Palette::for_mode(settings.theme, settings.accent);
        let sync = opts.api_base.as_deref().map(SyncClient::new);
        let mut app = Self {
            ws,
            settings,
            store,
            opts,
            current,
            history: History::new(blocks),
            search,
            editor: EditorState::default(),
            dbs: DbState::default(),
            palette: PaletteState::default(),
            toasts: Vec::new(),
            dirty: HashSet::new(),
            removed: Vec::new(),
            last_edit: 0.0,
            last_version: 0.0,
            show_settings: false,
            show_history: false,
            pal_from: pal,
            pal_to: pal,
            pal_changed_at: -10.0,
            last_applied: None,
            imports: Arc::default(),
            mentions_dirty: true,
            page_opened_at: -10.0,
            sync,
            sync_dirty: HashSet::new(),
            sync_removed: Vec::new(),
            sync_status: String::new(),
            signed_in: false,
            last_sync: -1000.0,
            sync_busy: false,
            login_email: String::new(),
            login_password: String::new(),
            cover_url: String::new(),
        };
        if app.sync.is_some() {
            app.sync_status = "Checking session…".into();
        }
        app
    }

    // ------------------------------------------------------------ helpers

    fn toast(&mut self, ctx: &Context, text: impl Into<String>) {
        let now = ctx.input(|i| i.time);
        self.toasts.push(Toast { text: text.into(), born: now, ttl: 3.2 });
        ctx.request_repaint();
    }

    fn target_palette(&self) -> Palette {
        Palette::for_mode(self.settings.theme, self.settings.accent)
    }

    fn mark_dirty(&mut self, id: Uuid, now: f64) {
        self.dirty.insert(id);
        self.sync_dirty.insert(id);
        self.last_edit = now;
    }

    fn open_page(&mut self, id: Uuid, now: f64) {
        if self.ws.page(id).is_none() {
            return;
        }
        self.flush();
        self.current = Some(id);
        let blocks = self.ws.page(id).map(|p| p.blocks.clone()).unwrap_or_default();
        self.history.reset(blocks);
        self.editor.reset();
        self.dbs = DbState::default();
        self.page_opened_at = now;
        self.settings.last_page = Some(id);
        self.store.save_settings(&self.settings);
        self.mentions_dirty = true;
    }

    fn new_page(&mut self, parent: Option<Uuid>, database: bool, now: f64) {
        let id = if database { self.ws.add_database(parent) } else { self.ws.add_page(parent) };
        if let Some(p) = self.ws.page(id) {
            self.search.index_page(p);
        }
        self.mark_dirty(id, now);
        self.open_page(id, now);
        if !database {
            self.editor.focus = None;
        }
    }

    fn delete_page(&mut self, id: Uuid, ctx: &Context) {
        let removed = self.ws.remove_page(id);
        for r in &removed {
            self.search.remove(*r);
            self.store.remove_page(*r);
            self.dirty.remove(r);
            self.sync_dirty.remove(r);
        }
        self.removed.extend(removed.iter().copied());
        self.sync_removed.extend(removed.iter().copied());
        if self.current.is_some_and(|c| removed.contains(&c)) {
            let next = self.ws.pages.first().map(|p| p.id);
            self.current = None;
            if let Some(n) = next {
                let now = ctx.input(|i| i.time);
                self.open_page(n, now);
            }
        }
        self.mentions_dirty = true;
        self.toast(ctx, "Page deleted");
    }

    /// Write dirty pages to storage.
    fn flush(&mut self) {
        if self.dirty.is_empty() {
            return;
        }
        let ids: Vec<Uuid> = self.dirty.drain().collect();
        for id in ids {
            if let Some(p) = self.ws.page(id) {
                self.store.save_page(p);
                self.search.index_page(p);
            }
        }
    }

    fn undo(&mut self, now: f64) {
        let Some(id) = self.current else { return };
        let Some(page) = self.ws.page(id) else { return };
        if let Some(prev) = self.history.undo(&page.blocks) {
            self.apply_blocks(id, prev, now);
        }
    }

    fn redo(&mut self, now: f64) {
        let Some(id) = self.current else { return };
        let Some(page) = self.ws.page(id) else { return };
        if let Some(next) = self.history.redo(&page.blocks) {
            self.apply_blocks(id, next, now);
        }
    }

    fn apply_blocks(&mut self, id: Uuid, blocks: Vec<Block>, now: f64) {
        if let Some(p) = self.ws.page_mut(id) {
            p.blocks = blocks;
            p.touch();
        }
        self.editor.heights.clear();
        self.mark_dirty(id, now);
    }

    fn export(&mut self, ctx: &Context, cmd: Cmd) {
        let Some(page) = self.current.and_then(|id| self.ws.page(id)) else { return };
        let name = (if page.title.trim().is_empty() { "Untitled" } else { page.title.trim() }).to_owned();
        let (file, mime, data): (String, &str, Vec<u8>) = match cmd {
            Cmd::ExportMd => {
                let mut md = format!("# {}\n\n", page.title);
                blocks_to_markdown(&page.blocks, 0, &mut md);
                (format!("{name}.md"), "text/markdown", md.into_bytes())
            }
            Cmd::ExportHtml => (format!("{name}.html"), "text/html", export::page_to_html(page).into_bytes()),
            Cmd::ExportJson => (format!("{name}.json"), "application/json", export::page_to_json(page).into_bytes()),
            _ => (format!("{name}.pdf"), "application/pdf", export::page_to_pdf(page)),
        };
        self.opts.platform.save_file(&file, mime, &data);
        self.toast(ctx, format!("Exported {file}"));
    }

    fn run_cmd(&mut self, cmd: Cmd, ctx: &Context) {
        let now = ctx.input(|i| i.time);
        match cmd {
            Cmd::NewPage => self.new_page(None, false, now),
            Cmd::NewDatabase => self.new_page(None, true, now),
            Cmd::ToggleTheme => {
                self.settings.theme = if self.settings.theme == ThemeMode::Light { ThemeMode::Dark } else { ThemeMode::Light };
                self.store.save_settings(&self.settings);
            }
            Cmd::ToggleSidebar => {
                self.settings.sidebar_open = !self.settings.sidebar_open;
                self.store.save_settings(&self.settings);
            }
            Cmd::ExportMd | Cmd::ExportHtml | Cmd::ExportJson | Cmd::ExportPdf => self.export(ctx, cmd),
            Cmd::ImportMd => self.opts.platform.request_import(ctx, self.imports.clone()),
            Cmd::DeletePage => {
                if let Some(id) = self.current {
                    self.delete_page(id, ctx);
                }
            }
            Cmd::ToggleFavorite => {
                if let Some(id) = self.current {
                    self.toggle_favorite(id, now);
                }
            }
            Cmd::OpenSettings => self.show_settings = true,
            Cmd::OpenHistory => self.show_history = true,
            Cmd::Undo => self.undo(now),
            Cmd::Redo => self.redo(now),
            Cmd::DownloadApp => ctx.open_url(egui::OpenUrl::new_tab(self.opts.download_url.clone())),
            Cmd::OpenWebApp => ctx.open_url(egui::OpenUrl::new_tab(format!("{}/app", self.opts.website_url))),
            Cmd::OpenWebsite => ctx.open_url(egui::OpenUrl::new_tab(self.opts.website_url.clone())),
        }
    }

    fn toggle_favorite(&mut self, id: Uuid, now: f64) {
        if let Some(p) = self.ws.page_mut(id) {
            p.favorite = !p.favorite;
            p.touch();
        }
        self.mark_dirty(id, now);
    }

    // ------------------------------------------------------------ frame setup

    fn setup_frame(&mut self, ctx: &Context, now: f64) {
        let anim_time = if self.settings.reduce_motion { 0.0 } else { 0.16 };
        if (ctx.style().animation_time - anim_time).abs() > 1e-6 {
            ctx.style_mut(|s| s.animation_time = anim_time);
        }
        let target = self.target_palette();
        if target != self.pal_to {
            self.pal_from = self.displayed_palette(now);
            self.pal_to = target;
            self.pal_changed_at = now;
        }
        let shown = self.displayed_palette(now);
        if self.last_applied != Some(shown) {
            theme::apply(ctx, &shown);
            self.last_applied = Some(shown);
        }
        if now - self.pal_changed_at < 0.35 {
            ctx.request_repaint();
        }
    }

    fn displayed_palette(&self, now: f64) -> Palette {
        if self.settings.reduce_motion {
            return self.pal_to;
        }
        let t = ((now - self.pal_changed_at) / 0.3).clamp(0.0, 1.0) as f32;
        self.pal_from.lerp(&self.pal_to, anim::ease_in_out_cubic(t))
    }

    fn handle_shortcuts(&mut self, ctx: &Context) {
        let now = ctx.input(|i| i.time);
        let (k, redo, undo, redo_y, side) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::K),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
                i.consume_key(Modifiers::COMMAND, Key::Z),
                i.consume_key(Modifiers::COMMAND, Key::Y),
                i.consume_key(Modifiers::COMMAND, Key::Backslash),
            )
        });
        if k {
            self.palette.open = !self.palette.open;
            self.palette.query.clear();
            self.palette.sel = 0;
            self.palette.just_opened = true;
        }
        if redo || redo_y {
            self.redo(now);
        } else if undo {
            self.undo(now);
        }
        if side {
            self.run_cmd(Cmd::ToggleSidebar, ctx);
        }
    }

    fn drain_imports(&mut self, ctx: &Context) {
        let files: Vec<(String, String)> = match self.imports.lock() {
            Ok(mut g) => std::mem::take(&mut *g),
            Err(_) => return,
        };
        let now = ctx.input(|i| i.time);
        for (name, content) in files {
            let stem = name.rsplit_once('.').map_or(name.as_str(), |(s, _)| s).to_owned();
            let id = if name.to_lowercase().ends_with(".json") {
                match serde_json::from_str::<Page>(&content) {
                    Ok(mut p) => {
                        p.id = Uuid::new_v4();
                        p.parent = None;
                        p.touch();
                        let id = p.id;
                        self.ws.pages.push(p);
                        id
                    }
                    Err(e) => {
                        self.toast(ctx, format!("Import failed: {e}"));
                        continue;
                    }
                }
            } else {
                self.ws.import_markdown(&stem, &content, None)
            };
            self.mark_dirty(id, now);
            self.open_page(id, now);
            self.toast(ctx, format!("Imported {name}"));
        }
    }

    // ------------------------------------------------------------ sync

    fn drain_sync(&mut self, ctx: &Context, now: f64) {
        let Some(client) = self.sync.clone() else { return };
        let (login, pulled, pushed) = match client.inbox.lock() {
            Ok(mut i) => (i.login.take(), i.pulled.take(), i.pushed.take()),
            Err(_) => return,
        };
        if let Some(res) = login {
            match res {
                Ok(auth) => {
                    if let Some(c) = self.sync.as_mut() {
                        c.token = Some(auth.token);
                    }
                    self.signed_in = true;
                    self.sync_status = format!("Signed in as {}", auth.user.email);
                    self.login_password.clear();
                    self.last_sync = -1000.0;
                }
                Err(e) => self.sync_status = format!("Sign-in failed: {e}"),
            }
        }
        if let Some(res) = pulled {
            match res {
                Ok(pull) => {
                    self.signed_in = true;
                    self.merge_remote(pull.pages, now);
                    let pages: Vec<&Page> = self.sync_dirty.iter().filter_map(|id| self.ws.page(*id)).collect();
                    if !pages.is_empty() || !self.sync_removed.is_empty() {
                        client.push(&pages, &self.sync_removed, ctx);
                    } else {
                        self.sync_busy = false;
                        self.sync_status = "Synced".into();
                    }
                }
                Err(e) => {
                    self.sync_busy = false;
                    self.sync_status = if e.contains("401") || e.to_lowercase().contains("unauthor") {
                        self.signed_in = false;
                        "Not signed in".into()
                    } else {
                        format!("Sync error: {e}")
                    };
                }
            }
        }
        if let Some(res) = pushed {
            self.sync_busy = false;
            match res {
                Ok(()) => {
                    self.sync_dirty.clear();
                    self.sync_removed.clear();
                    self.sync_status = "Synced".into();
                }
                Err(e) => self.sync_status = format!("Push failed: {e}"),
            }
        }
        let should = self.signed_in || self.last_sync < 0.0;
        if should && !self.sync_busy && now - self.last_sync > 30.0 && self.dirty.is_empty() {
            self.last_sync = now;
            self.sync_busy = true;
            client.pull(ctx);
        }
        if self.sync.is_some() {
            ctx.request_repaint_after(Duration::from_secs(5));
        }
    }

    fn merge_remote(&mut self, remote: Vec<nw_api_types::SyncPage>, now: f64) {
        for sp in remote {
            if sp.deleted {
                if self.ws.page(sp.id).is_some_and(|p| p.updated < sp.updated) {
                    let removed = self.ws.remove_page(sp.id);
                    for r in removed {
                        self.search.remove(r);
                        self.store.remove_page(r);
                    }
                    if self.current == Some(sp.id) {
                        self.current = self.ws.pages.first().map(|p| p.id);
                    }
                }
                continue;
            }
            let Ok(page) = serde_json::from_value::<Page>(sp.data) else { continue };
            let id = page.id;
            match self.ws.index_of(id) {
                Some(ix) => {
                    let mut merged = merge_pages(&self.ws.pages[ix], &page);
                    let newer_db = page.database.is_some() && page.updated > self.ws.pages[ix].updated;
                    if merged.blocks != self.ws.pages[ix].blocks || merged.title != self.ws.pages[ix].title || newer_db {
                        if newer_db {
                            merged.database = page.database.clone();
                        } else if merged.database.is_none() {
                            merged.database = self.ws.pages[ix].database.clone();
                        }
                        self.ws.pages[ix] = merged;
                        self.dirty.insert(id);
                        if self.current == Some(id) {
                            let blocks = self.ws.pages[ix].blocks.clone();
                            self.history.reset(blocks);
                            self.editor.heights.clear();
                        }
                    }
                }
                None => {
                    self.ws.pages.push(page);
                    self.dirty.insert(id);
                }
            }
        }
        self.mentions_dirty = true;
        self.last_edit = now - 10.0;
    }

    // ------------------------------------------------------------ sidebar

    fn nav_item(ui: &mut Ui, pal: &Palette, icon: &str, label: &str, hint: &str) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
        let a = anim::ease_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), anim::FAST);
        if a > 0.01 {
            ui.painter().rect_filled(rect, 6.0, pal.hover.gamma_multiply(a));
        }
        ui.painter()
            .text(rect.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, format!("{icon}  {label}"), FontId::proportional(14.0), pal.text);
        if !hint.is_empty() {
            ui.painter()
                .text(rect.right_center() - vec2(8.0, 0.0), Align2::RIGHT_CENTER, hint, FontId::proportional(11.0), pal.dim);
        }
        resp
    }

    fn page_row(ui: &mut Ui, ws: &Workspace, p: &Page, current: Option<Uuid>, pal: &Palette, acts: &mut Vec<SideAction>) {
        let ctx = ui.ctx().clone();
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click_and_drag());
        let selected = current == Some(p.id);
        let a = anim::ease_bool(&ctx, resp.id.with("hover"), resp.hovered(), anim::FAST);
        let sel_a = anim::ease_bool(&ctx, resp.id.with("sel"), selected, anim::NORMAL);
        if sel_a > 0.01 {
            ui.painter().rect_filled(rect, 6.0, pal.active.gamma_multiply(sel_a));
        } else if a > 0.01 {
            ui.painter().rect_filled(rect, 6.0, pal.hover.gamma_multiply(a));
        }
        let title = if p.title.is_empty() { "Untitled" } else { p.title.as_str() };
        let clip = ui.painter().with_clip_rect(Rect::from_min_max(rect.min, pos2(rect.right() - 30.0, rect.bottom())));
        clip.text(
            rect.left_center() + vec2(6.0, 0.0),
            Align2::LEFT_CENTER,
            format!("{}  {}", p.icon, title),
            FontId::proportional(14.0),
            if selected { pal.text } else { pal.text.gamma_multiply(0.85) },
        );
        if a > 0.05 {
            let plus = Rect::from_center_size(rect.right_center() - vec2(14.0, 0.0), vec2(20.0, 20.0));
            let pr = ui.interact(plus, resp.id.with("plus"), Sense::click());
            if pr.hovered() {
                ui.painter().rect_filled(plus, 4.0, pal.active);
            }
            ui.painter().text(plus.center(), Align2::CENTER_CENTER, "＋", FontId::proportional(13.0), pal.dim.gamma_multiply(a));
            if pr.clicked() {
                acts.push(SideAction::AddChild(p.id));
            }
        }
        if resp.clicked() {
            acts.push(SideAction::Open(p.id));
        }
        if resp.drag_started() {
            resp.dnd_set_drag_payload(PageDrag(p.id));
        }
        resp.context_menu(|ui| {
            if ui.button("＋ Add subpage").clicked() {
                acts.push(SideAction::AddChild(p.id));
                ui.close_menu();
            }
            if ui.button(if p.favorite { "☆ Remove favorite" } else { "★ Add to favorites" }).clicked() {
                acts.push(SideAction::Favorite(p.id));
                ui.close_menu();
            }
            if ui.button("🗑 Delete").clicked() {
                acts.push(SideAction::Delete(p.id));
                ui.close_menu();
            }
        });
        // Drop target: top quarter = before, bottom quarter = after, middle = nest.
        if let (Some(d), Some(pos)) = (egui::DragAndDrop::payload::<PageDrag>(&ctx), ctx.pointer_latest_pos()) {
            if d.0 != p.id && rect.contains(pos) {
                let rel = (pos.y - rect.top()) / rect.height();
                let released = ctx.input(|i| i.pointer.any_released());
                let stroke = egui::Stroke::new(2.0, pal.accent);
                if rel < 0.25 {
                    ui.painter().hline(rect.x_range(), rect.top(), stroke);
                    if released {
                        acts.push(SideAction::Move { id: d.0, parent: p.parent, before: Some(p.id) });
                    }
                } else if rel > 0.75 {
                    ui.painter().hline(rect.x_range(), rect.bottom(), stroke);
                    if released {
                        let sibs = ws.children(p.parent);
                        let next = sibs.iter().position(|s| *s == p.id).and_then(|i| sibs.get(i + 1).copied());
                        acts.push(SideAction::Move { id: d.0, parent: p.parent, before: next });
                    }
                } else {
                    ui.painter().rect_filled(rect, 6.0, pal.accent_soft);
                    if released {
                        acts.push(SideAction::Move { id: d.0, parent: Some(p.id), before: None });
                    }
                }
            }
        }
    }

    fn tree_row(ui: &mut Ui, ws: &Workspace, id: Uuid, current: Option<Uuid>, pal: &Palette, acts: &mut Vec<SideAction>, salt: &str) {
        let Some(p) = ws.page(id) else { return };
        let kids = ws.children(Some(id));
        if kids.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(ui.spacing().indent);
                Self::page_row(ui, ws, p, current, pal, acts);
            });
        } else {
            let cid = ui.make_persistent_id((salt, "tree", id));
            CollapsingState::load_with_default_open(ui.ctx(), cid, false)
                .show_header(ui, |ui| Self::page_row(ui, ws, p, current, pal, acts))
                .body(|ui| {
                    for k in kids {
                        Self::tree_row(ui, ws, k, current, pal, acts, salt);
                    }
                });
        }
    }

    fn sidebar_ui(&mut self, ui: &mut Ui, pal: &Palette, now: f64) {
        let ctx = ui.ctx().clone();
        let mut acts: Vec<SideAction> = Vec::new();
        ui.horizontal(|ui| {
            ui.label(RichText::new("📓 My Workspace").strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("«").on_hover_text("Collapse sidebar (Ctrl+\\)").clicked() {
                    self.settings.sidebar_open = false;
                    self.store.save_settings(&self.settings);
                }
            });
        });
        ui.add_space(6.0);
        if Self::nav_item(ui, pal, "🔍", "Search", "Ctrl K").clicked() {
            self.palette.open = true;
            self.palette.just_opened = true;
            self.palette.query.clear();
        }
        if Self::nav_item(ui, pal, "⚙", "Settings", "").clicked() {
            self.show_settings = true;
        }
        if Self::nav_item(ui, pal, "＋", "New page", "").clicked() {
            self.new_page(None, false, now);
        }
        if Self::nav_item(ui, pal, "🗂", "New database", "").clicked() {
            self.new_page(None, true, now);
        }
        ui.add_space(8.0);
        let footer_h = 56.0;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(ui.available_height() - footer_h)
            .show(ui, |ui| {
                let favs: Vec<Uuid> = self.ws.pages.iter().filter(|p| p.favorite).map(|p| p.id).collect();
                if !favs.is_empty() {
                    ui.label(RichText::new("Favorites").size(12.0).color(pal.dim));
                    for id in favs {
                        if let Some(p) = self.ws.page(id) {
                            ui.horizontal(|ui| {
                                ui.add_space(ui.spacing().indent);
                                Self::page_row(ui, &self.ws, p, self.current, pal, &mut acts);
                            });
                        }
                    }
                    ui.add_space(10.0);
                }
                let hdr = ui.label(RichText::new("Pages").size(12.0).color(pal.dim));
                // Dropping on the header moves a page to the top level.
                if let (Some(d), Some(pos)) = (egui::DragAndDrop::payload::<PageDrag>(&ctx), ctx.pointer_latest_pos()) {
                    if hdr.rect.expand2(vec2(40.0, 4.0)).contains(pos) {
                        ui.painter().rect_filled(hdr.rect.expand2(vec2(40.0, 2.0)), 4.0, pal.accent_soft);
                        if ctx.input(|i| i.pointer.any_released()) {
                            acts.push(SideAction::Move { id: d.0, parent: None, before: None });
                        }
                    }
                }
                for id in self.ws.children(None) {
                    Self::tree_row(ui, &self.ws, id, self.current, pal, &mut acts, "main");
                }
            });
        ui.separator();
        if self.opts.is_web {
            if Self::nav_item(ui, pal, "⬇", "Download desktop app", "").clicked() {
                ctx.open_url(egui::OpenUrl::new_tab(self.opts.download_url.clone()));
            }
            if Self::nav_item(ui, pal, "🏠", "Back to website", "").clicked() {
                ctx.open_url(egui::OpenUrl::new_tab(self.opts.website_url.clone()));
            }
        } else if Self::nav_item(ui, pal, "🌐", "Open web app", "").clicked() {
            ctx.open_url(egui::OpenUrl::new_tab(format!("{}/app", self.opts.website_url)));
        }

        // Drag ghost for page moves
        if let (Some(d), Some(pos)) = (egui::DragAndDrop::payload::<PageDrag>(&ctx), ctx.pointer_interact_pos()) {
            if let Some(p) = self.ws.page(d.0) {
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                egui::Area::new(Id::new("page-drag-ghost"))
                    .order(egui::Order::Tooltip)
                    .fixed_pos(pos + vec2(14.0, 10.0))
                    .interactable(false)
                    .show(&ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(format!("{} {}", p.icon, p.title)));
                    });
            }
        }

        for a in acts {
            match a {
                SideAction::Open(id) => self.open_page(id, now),
                SideAction::AddChild(id) => self.new_page(Some(id), false, now),
                SideAction::Delete(id) => self.delete_page(id, &ctx),
                SideAction::Favorite(id) => self.toggle_favorite(id, now),
                SideAction::Move { id, parent, before } => {
                    if self.ws.move_page(id, parent, before) {
                        self.mark_dirty(id, now);
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------ central

    fn topbar(&mut self, ui: &mut Ui, pal: &Palette, now: f64) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            if !self.settings.sidebar_open && ui.button("☰").on_hover_text("Open sidebar").clicked() {
                self.settings.sidebar_open = true;
                self.store.save_settings(&self.settings);
            }
            if let Some(id) = self.current {
                let crumbs = self.ws.breadcrumb(id);
                let n = crumbs.len();
                for (i, (cid, label)) in crumbs.into_iter().enumerate() {
                    if ui.add(egui::Button::new(RichText::new(label).size(13.0)).frame(false)).clicked() && cid != id {
                        self.open_page(cid, now);
                    }
                    if i + 1 < n {
                        ui.label(RichText::new("/").color(pal.dim));
                    }
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(8.0);
                let dark = pal.is_dark();
                if ui.button(if dark { "☀" } else { "🌙" }).on_hover_text("Toggle theme").clicked() {
                    self.run_cmd(Cmd::ToggleTheme, &ctx);
                }
                ui.menu_button("⋯", |ui| {
                    for (label, cmd) in [
                        ("Export Markdown", Cmd::ExportMd),
                        ("Export HTML", Cmd::ExportHtml),
                        ("Export JSON", Cmd::ExportJson),
                        ("Export PDF", Cmd::ExportPdf),
                        ("Import file…", Cmd::ImportMd),
                        ("Version history", Cmd::OpenHistory),
                        ("Delete page", Cmd::DeletePage),
                    ] {
                        if ui.button(label).clicked() {
                            self.run_cmd(cmd, &ctx);
                            ui.close_menu();
                        }
                    }
                });
                let fav = self.current.and_then(|id| self.ws.page(id)).is_some_and(|p| p.favorite);
                if ui.button(if fav { "★" } else { "☆" }).on_hover_text("Favorite").clicked() {
                    self.run_cmd(Cmd::ToggleFavorite, &ctx);
                }
                if self.opts.is_web && ui.button("⬇ Desktop app").clicked() {
                    self.run_cmd(Cmd::DownloadApp, &ctx);
                }
                if self.sync.is_some() {
                    let text = if self.signed_in { "● synced" } else { "○ offline" };
                    ui.label(RichText::new(text).size(12.0).color(pal.dim)).on_hover_text(&self.sync_status);
                }
            });
        });
        ui.separator();
    }

    fn page_view(&mut self, ui: &mut Ui, pal: &Palette, now: f64) {
        let Some(id) = self.current else {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() / 3.0);
                ui.label(RichText::new("No page selected").size(20.0).color(pal.dim));
                if ui.button("Create a page").clicked() {
                    self.new_page(None, false, now);
                }
            });
            return;
        };
        if self.mentions_dirty {
            self.editor.mention_pages = self.ws.pages.iter().map(|p| (p.id, p.title.clone())).collect();
            self.mentions_dirty = false;
        }
        let open_a = anim::ease_out_cubic(((now - self.page_opened_at) / 0.28) as f32);
        if open_a < 1.0 {
            ui.ctx().request_repaint();
        }
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_opacity(open_a);
            ui.add_space((1.0 - open_a) * 10.0);
            let avail = ui.available_width();
            // Cover
            let cover = self.ws.page(id).and_then(|p| p.cover.clone());
            if let Some(c) = &cover {
                let (rect, _) = ui.allocate_exact_size(vec2(avail, 170.0), Sense::hover());
                match c {
                    Cover::Color(rgb) => {
                        ui.painter().rect_filled(rect, 0.0, Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                    }
                    Cover::Url(u) => {
                        ui.put(rect, egui::Image::new(u.clone()).fit_to_exact_size(rect.size()).maintain_aspect_ratio(false));
                    }
                }
            } else {
                ui.add_space(40.0);
            }

            let content_w = (avail - 32.0).clamp(280.0, 820.0);
            let left = ((avail - content_w) / 2.0).max(0.0);
            let area = ui.available_rect_before_wrap();
            let col = Rect::from_min_size(pos2(area.left() + left, area.top()), vec2(content_w, area.height().max(400.0)));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(col).layout(Layout::top_down(Align::Min)));
            let c = &mut child;

            // icon + header actions
            let (icon, is_db) = self.ws.page(id).map_or((String::new(), false), |p| (p.icon.clone(), p.database.is_some()));
            c.add_space(if cover.is_some() { 14.0 } else { 0.0 });
            c.label(RichText::new(&icon).size(54.0));
            c.horizontal(|ui| {
                ui.menu_button("😀 Icon", |ui| {
                    egui::Grid::new("emoji-grid").show(ui, |ui| {
                        const EMOJI: [&str; 24] = [
                            "📄", "📝", "📚", "📌", "💡", "🎯", "🚀", "🌱", "✅", "🗂", "📅", "🔥", "⭐", "🏠", "💼", "🧠", "🎨", "🧪", "🛠",
                            "📈", "❤", "🌍", "🔒", "☕",
                        ];
                        for (i, e) in EMOJI.iter().enumerate() {
                            if ui.button(*e).clicked() {
                                if let Some(p) = self.ws.page_mut(id) {
                                    p.icon = (*e).to_owned();
                                    p.touch();
                                }
                                self.mark_dirty(id, now);
                                ui.close_menu();
                            }
                            if i % 8 == 7 {
                                ui.end_row();
                            }
                        }
                    });
                });
                ui.menu_button("🖼 Cover", |ui| {
                    ui.horizontal(|ui| {
                        for rgb in [[250, 222, 201], [211, 229, 239], [219, 237, 219], [232, 222, 238], [253, 236, 200], [60, 60, 70]] {
                            let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
                            ui.painter().rect_filled(r, 6.0, Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                            if resp.clicked() {
                                if let Some(p) = self.ws.page_mut(id) {
                                    p.cover = Some(Cover::Color(rgb));
                                    p.touch();
                                }
                                self.mark_dirty(id, now);
                                ui.close_menu();
                            }
                        }
                    });
                    ui.add(egui::TextEdit::singleline(&mut self.cover_url).hint_text("Image URL…").desired_width(220.0));
                    if ui.button("Use image URL").clicked() && !self.cover_url.trim().is_empty() {
                        let url = self.cover_url.trim().to_owned();
                        if let Some(p) = self.ws.page_mut(id) {
                            p.cover = Some(Cover::Url(url));
                            p.touch();
                        }
                        self.mark_dirty(id, now);
                        ui.close_menu();
                    }
                    if ui.button("Remove cover").clicked() {
                        if let Some(p) = self.ws.page_mut(id) {
                            p.cover = None;
                            p.touch();
                        }
                        self.mark_dirty(id, now);
                        ui.close_menu();
                    }
                });
            });

            // title
            let mut title = self.ws.page(id).map(|p| p.title.clone()).unwrap_or_default();
            let tr = c.add(
                egui::TextEdit::singleline(&mut title)
                    .id(Id::new(("page-title", id)))
                    .frame(false)
                    .font(fonts::bold(40.0))
                    .desired_width(f32::INFINITY)
                    .hint_text(RichText::new("Untitled").size(40.0).color(pal.dim.gamma_multiply(0.6))),
            );
            if tr.changed() {
                if let Some(p) = self.ws.page_mut(id) {
                    p.title = title;
                    p.touch();
                }
                self.mark_dirty(id, now);
                self.mentions_dirty = true;
            }
            if tr.lost_focus() && ctx.input(|i| i.key_pressed(Key::Enter)) {
                if let Some(first) = self.ws.page(id).and_then(|p| p.blocks.first()) {
                    self.editor.focus = Some((first.id, 0));
                }
            }
            c.add_space(10.0);

            // body
            let mut changed = false;
            let mut db_changed = false;
            if is_db {
                if let Some(db) = self.ws.page_mut(id).and_then(|p| p.database.as_mut()) {
                    db_changed = db_view::show_database(c, db, &mut self.dbs, pal);
                }
            } else if let Some(page) = self.ws.page_mut(id) {
                changed = editor::show_page_blocks(c, &mut page.blocks, &mut self.editor, pal);
                if !self.editor.tombstones.is_empty() {
                    let ts = now_ms();
                    for t in self.editor.tombstones.drain(..) {
                        page.tombstones.insert(t, ts);
                    }
                }
            }
            if changed || db_changed {
                if let Some(p) = self.ws.page_mut(id) {
                    p.touch();
                }
                self.mark_dirty(id, now);
                if changed {
                    self.history.mark_changed(now_ms());
                }
            }
            if let Some(target) = self.editor.open_page.take() {
                self.open_page(target, now);
            }
            let used = child.min_rect();
            ui.allocate_rect(used, Sense::hover());
        });
        if let Some(p) = self.ws.page(id) {
            self.history.settle(&p.blocks, now_ms());
        }
    }

    // ------------------------------------------------------------ overlays

    fn palette_items(&self) -> Vec<Item> {
        let q = self.palette.query.trim();
        let mut pages: Vec<(i32, Uuid, String)> = Vec::new();
        if q.is_empty() {
            for p in self.ws.pages.iter().take(6) {
                pages.push((0, p.id, format!("{} {}", p.icon, p.title)));
            }
        } else {
            for p in &self.ws.pages {
                if let Some(s) = fuzzy_score(q, &p.title) {
                    pages.push((s + 100, p.id, format!("{} {}", p.icon, p.title)));
                }
            }
            for (id, s) in self.search.query(q) {
                if !pages.iter().any(|x| x.1 == id) {
                    if let Some(p) = self.ws.page(id) {
                        pages.push((s, id, format!("{} {}  · in content", p.icon, p.title)));
                    }
                }
            }
        }
        pages.sort_by(|a, b| b.0.cmp(&a.0));
        let mut out: Vec<Item> = pages.into_iter().take(8).map(|(_, id, t)| Item::Page(id, t)).collect();
        let mut cmds: Vec<(i32, &(Cmd, &str, &str))> = CMDS
            .iter()
            .filter(|(c, _, _)| match c {
                Cmd::DownloadApp => self.opts.is_web,
                Cmd::OpenWebApp => !self.opts.is_web,
                _ => true,
            })
            .filter_map(|c| fuzzy_score(q, c.1).map(|s| (s, c)))
            .collect();
        cmds.sort_by(|a, b| b.0.cmp(&a.0));
        out.extend(cmds.into_iter().take(8).map(|(_, (c, l, i))| Item::Cmd(*c, *l, *i)));
        out
    }

    fn palette_ui(&mut self, ctx: &Context, pal: &Palette, now: f64) {
        let a = anim::ease_bool(ctx, Id::new("palette-anim"), self.palette.open, anim::NORMAL);
        if a <= 0.001 {
            return;
        }
        let screen = ctx.screen_rect();
        // scrim
        egui::Area::new(Id::new("palette-scrim")).order(egui::Order::Middle).fixed_pos(screen.min).show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha((90.0 * a) as u8));
            if resp.clicked() {
                self.palette.open = false;
            }
        });
        if !self.palette.open {
            return;
        }
        let items = self.palette_items();
        let (down, up, enter, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if down {
            self.palette.sel += 1;
        }
        if up {
            self.palette.sel = self.palette.sel.saturating_sub(1);
        }
        self.palette.sel = self.palette.sel.min(items.len().saturating_sub(1));
        if esc {
            self.palette.open = false;
        }
        let mut chosen: Option<usize> = if enter && !items.is_empty() { Some(self.palette.sel) } else { None };
        let w = 560.0 * (0.97 + 0.03 * a);
        let pos = pos2(screen.center().x - w / 2.0, screen.top() + 90.0 - (1.0 - a) * 12.0);
        egui::Area::new(Id::new("palette")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
            ui.set_opacity(a);
            egui::Frame::popup(ui.style()).corner_radius(12).inner_margin(10).show(ui, |ui| {
                ui.set_width(w);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.palette.query)
                        .hint_text("Search pages or run a command…")
                        .font(FontId::proportional(17.0))
                        .frame(false)
                        .desired_width(f32::INFINITY),
                );
                if self.palette.just_opened {
                    r.request_focus();
                    self.palette.just_opened = false;
                }
                if r.changed() {
                    self.palette.sel = 0;
                }
                ui.separator();
                egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                    let mut last_was_page = true;
                    for (i, item) in items.iter().enumerate() {
                        if matches!(item, Item::Cmd(..)) && last_was_page {
                            ui.label(RichText::new("Commands").size(11.0).color(pal.dim));
                            last_was_page = false;
                        } else if i == 0 {
                            ui.label(RichText::new("Pages").size(11.0).color(pal.dim));
                        }
                        let (rect, resp) = ui.allocate_exact_size(vec2(w, 32.0), Sense::click());
                        if i == self.palette.sel || resp.hovered() {
                            ui.painter().rect_filled(rect, 6.0, pal.hover);
                        }
                        if i == self.palette.sel && (down || up) {
                            resp.scroll_to_me(None);
                        }
                        let label = match item {
                            Item::Page(_, t) => t.clone(),
                            Item::Cmd(_, l, ic) => format!("{ic}  {l}"),
                        };
                        ui.painter()
                            .text(rect.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(14.5), pal.text);
                        if resp.clicked() {
                            chosen = Some(i);
                        }
                    }
                    if items.is_empty() {
                        ui.label(RichText::new("No results").color(pal.dim));
                    }
                });
            });
        });
        if let Some(i) = chosen {
            if let Some(item) = items.into_iter().nth(i) {
                self.palette.open = false;
                match item {
                    Item::Page(id, _) => self.open_page(id, now),
                    Item::Cmd(c, _, _) => self.run_cmd(c, ctx),
                }
            }
        }
    }

    fn settings_ui(&mut self, ui: &mut Ui, ctx: &Context) {
        let mut changed = false;
        ui.label(RichText::new("Appearance").strong());
        ui.horizontal(|ui| {
            for (label, mode) in [("Light", ThemeMode::Light), ("Dark", ThemeMode::Dark), ("High contrast", ThemeMode::HighContrast)] {
                changed |= ui.selectable_value(&mut self.settings.theme, mode, label).changed();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Accent");
            for (name, rgb) in ACCENTS {
                let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                ui.painter().circle_filled(r.center(), 9.0, Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                if self.settings.accent == *rgb {
                    ui.painter().circle_stroke(r.center(), 11.0, egui::Stroke::new(2.0, ui.visuals().text_color()));
                }
                if resp.on_hover_text(*name).clicked() {
                    self.settings.accent = *rgb;
                    changed = true;
                }
            }
        });
        changed |= ui.checkbox(&mut self.settings.reduce_motion, "Reduce motion").changed();
        if changed {
            self.store.save_settings(&self.settings);
        }
        ui.separator();
        ui.label(RichText::new("Shortcuts").strong());
        ui.label("Ctrl+K palette · Ctrl+Z / Ctrl+Shift+Z undo / redo · Ctrl+\\ sidebar");
        ui.label("Ctrl+B bold · Ctrl+I italic · Ctrl+E code · Ctrl+Shift+S strike · Ctrl+H highlight");
        ui.label("Type / for blocks, @ to link pages, # - [] > ``` --- for markdown shortcuts");
        if self.sync.is_some() {
            ui.separator();
            ui.label(RichText::new("Sync").strong());
            ui.label(if self.sync_status.is_empty() { "Not signed in" } else { self.sync_status.as_str() });
            if !self.signed_in {
                if !self.opts.is_web {
                    ui.add(egui::TextEdit::singleline(&mut self.login_email).hint_text("Email"));
                    ui.add(egui::TextEdit::singleline(&mut self.login_password).password(true).hint_text("Password"));
                    if ui.button("Sign in").clicked() {
                        if let Some(c) = &self.sync {
                            c.login(&self.login_email, &self.login_password, ctx);
                            self.sync_status = "Signing in…".into();
                        }
                    }
                } else if ui.link("Sign in on the website").clicked() {
                    ctx.open_url(egui::OpenUrl::new_tab(format!("{}/login?next=/app", self.opts.website_url)));
                }
            }
        }
        ui.separator();
        ui.hyperlink_to("Website", self.opts.website_url.clone());
        ui.hyperlink_to("Download desktop app", self.opts.download_url.clone());
    }

    fn history_ui(&mut self, ui: &mut Ui, ctx: &Context, now: f64) {
        let Some(id) = self.current else { return };
        if ui.button("Save snapshot now").clicked() {
            self.ws.snapshot_version(id);
            self.store.save_versions(&self.ws);
        }
        ui.separator();
        let versions: Vec<(u64, String, usize)> = self
            .ws
            .versions_of(id)
            .iter()
            .map(|v| (v.ts, v.title.clone(), nw_core::block::count(&v.blocks)))
            .collect();
        if versions.is_empty() {
            ui.label("No versions yet — snapshots are taken automatically while you edit.");
        }
        let mut restore = None;
        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
            for (ts, title, n) in &versions {
                ui.horizontal(|ui| {
                    ui.label(format!("{}  ·  {} blocks", fmt_ts(*ts), n));
                    ui.label(RichText::new(title).weak());
                    if ui.small_button("Restore").clicked() {
                        restore = Some(*ts);
                    }
                });
            }
        });
        if let Some(ts) = restore {
            if self.ws.restore_version(id, ts) {
                let blocks = self.ws.page(id).map(|p| p.blocks.clone()).unwrap_or_default();
                self.history.reset(blocks);
                self.editor.heights.clear();
                self.mark_dirty(id, now);
                self.toast(ctx, "Version restored");
            }
        }
    }

    fn toasts_ui(&mut self, ctx: &Context, pal: &Palette) {
        let now = ctx.input(|i| i.time);
        self.toasts.retain(|t| now - t.born < t.ttl);
        let mut next_wake: Option<f64> = None;
        for (i, t) in self.toasts.iter().enumerate() {
            let age = now - t.born;
            let fade_in = anim::ease_out_back((age / 0.25) as f32).min(1.0);
            let fade_out = ((t.ttl - age) / 0.3).clamp(0.0, 1.0) as f32;
            let a = fade_in.min(fade_out);
            egui::Area::new(Id::new(("toast", i)))
                .order(egui::Order::Tooltip)
                .anchor(Align2::RIGHT_BOTTOM, vec2(-16.0 + (1.0 - a) * 30.0, -16.0 - i as f32 * 48.0))
                .interactable(false)
                .show(ctx, |ui| {
                    ui.set_opacity(a);
                    egui::Frame::popup(ui.style()).fill(pal.text).corner_radius(8).show(ui, |ui| {
                        ui.label(RichText::new(&t.text).color(pal.bg));
                    });
                });
            let wake = if age < 0.3 || t.ttl - age < 0.35 { 0.0 } else { t.ttl - 0.35 - age };
            next_wake = Some(next_wake.map_or(wake, |w| w.min(wake)));
        }
        match next_wake {
            Some(w) if w <= 0.0 => ctx.request_repaint(),
            Some(w) => ctx.request_repaint_after(Duration::from_secs_f64(w)),
            None => {}
        }
    }
}

impl eframe::App for NotionApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        let now = ctx.input(|i| i.time);
        self.setup_frame(ctx, now);
        let pal = self.displayed_palette(now);
        self.handle_shortcuts(ctx);
        self.drain_imports(ctx);
        self.drain_sync(ctx, now);

        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(256.0)
            .width_range(200.0..=400.0)
            .frame(egui::Frame::new().fill(pal.sidebar).inner_margin(8))
            .show_animated(ctx, self.settings.sidebar_open, |ui| self.sidebar_ui(ui, &pal, now));

        egui::CentralPanel::default().frame(egui::Frame::new().fill(pal.bg)).show(ctx, |ui| {
            self.topbar(ui, &pal, now);
            self.page_view(ui, &pal, now);
        });

        self.palette_ui(ctx, &pal, now);

        let mut open = self.show_settings;
        egui::Window::new("Settings").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| self.settings_ui(ui, ctx));
        self.show_settings = open;
        let mut open = self.show_history;
        egui::Window::new("Version history").open(&mut open).collapsible(false).show(ctx, |ui| self.history_ui(ui, ctx, now));
        self.show_history = open;

        self.toasts_ui(ctx, &pal);

        // Autosave (debounced) and periodic version snapshots.
        if !self.dirty.is_empty() {
            if now - self.last_edit > 0.8 {
                self.flush();
            } else {
                ctx.request_repaint_after(Duration::from_millis(900));
            }
        }
        if let Some(id) = self.current {
            if now - self.last_version > 120.0 {
                self.last_version = now;
                self.ws.snapshot_version(id);
                self.store.save_versions(&self.ws);
            }
        }
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.flush();
        self.store.save_settings(&self.settings);
    }
}
