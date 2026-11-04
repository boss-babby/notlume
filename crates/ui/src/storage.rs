//! Storage abstraction. Native uses SQLite (`app-native`), web uses
//! `localStorage` (`app-web`). The UI only sees this trait.

use nw_core::{Page, Settings, Uuid, Workspace};

pub trait Persistence {
    /// Load everything. Returning `None` means "first run".
    fn load(&mut self) -> Option<(Workspace, Settings)>;
    fn save_page(&mut self, page: &Page);
    fn remove_page(&mut self, id: Uuid);
    fn save_settings(&mut self, settings: &Settings);
    /// Persist version history (called less often than `save_page`).
    fn save_versions(&mut self, ws: &Workspace);
}

/// Non-persistent store, used in tests and as a fallback.
#[derive(Default)]
pub struct MemoryStore {
    pub ws: Option<Workspace>,
    pub settings: Option<Settings>,
}

impl Persistence for MemoryStore {
    fn load(&mut self) -> Option<(Workspace, Settings)> {
        Some((self.ws.clone()?, self.settings.clone().unwrap_or_default()))
    }
    fn save_page(&mut self, _page: &Page) {}
    fn remove_page(&mut self, _id: Uuid) {}
    fn save_settings(&mut self, settings: &Settings) {
        self.settings = Some(settings.clone());
    }
    fn save_versions(&mut self, _ws: &Workspace) {}
}
