//! SQLite persistence: one row per page (JSON), plus settings and versions.

use nw_core::{Page, Settings, Uuid, Workspace};
use nw_ui::Persistence;
use rusqlite::{params, Connection};
use std::path::Path;

pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS pages (
                 id TEXT PRIMARY KEY, updated INTEGER NOT NULL, json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS settings (
                 key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS versions (
                 page TEXT NOT NULL, ts INTEGER NOT NULL, json TEXT NOT NULL,
                 PRIMARY KEY (page, ts));",
        )?;
        Ok(Self { conn })
    }
}

impl Persistence for SqliteStore {
    fn load(&mut self) -> Option<(Workspace, Settings)> {
        let mut ws = Workspace::default();
        {
            let mut stmt = self.conn.prepare("SELECT json FROM pages ORDER BY rowid").ok()?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).ok()?;
            for json in rows.flatten() {
                match serde_json::from_str::<Page>(&json) {
                    Ok(p) => ws.pages.push(p),
                    Err(e) => log::warn!("skipping unreadable page: {e}"),
                }
            }
        }
        let settings: Option<Settings> = self
            .conn
            .query_row("SELECT value FROM settings WHERE key = 'settings'", [], |r| r.get::<_, String>(0))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());
        if ws.pages.is_empty() && settings.is_none() {
            return None;
        }
        if let Ok(mut stmt) = self.conn.prepare("SELECT json FROM versions ORDER BY ts") {
            if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
                for json in rows.flatten() {
                    if let Ok(v) = serde_json::from_str(&json) {
                        ws.versions.push(v);
                    }
                }
            }
        }
        Some((ws, settings.unwrap_or_default()))
    }

    fn save_page(&mut self, page: &Page) {
        let Ok(json) = serde_json::to_string(page) else { return };
        // `INSERT .. ON CONFLICT DO UPDATE` keeps the rowid, so page order is stable.
        if let Err(e) = self.conn.execute(
            "INSERT INTO pages (id, updated, json) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET updated = excluded.updated, json = excluded.json",
            params![page.id.to_string(), page.updated as i64, json],
        ) {
            log::error!("save_page failed: {e}");
        }
    }

    fn remove_page(&mut self, id: Uuid) {
        let id = id.to_string();
        let _ = self.conn.execute("DELETE FROM pages WHERE id = ?1", params![id]);
        let _ = self.conn.execute("DELETE FROM versions WHERE page = ?1", params![id]);
    }

    fn save_settings(&mut self, settings: &Settings) {
        if let Ok(json) = serde_json::to_string(settings) {
            let _ = self.conn.execute(
                "INSERT INTO settings (key, value) VALUES ('settings', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![json],
            );
        }
    }

    fn save_versions(&mut self, ws: &Workspace) {
        let Ok(tx) = self.conn.transaction() else { return };
        let _ = tx.execute("DELETE FROM versions", []);
        for v in &ws.versions {
            if let Ok(json) = serde_json::to_string(v) {
                let _ = tx.execute(
                    "INSERT OR REPLACE INTO versions (page, ts, json) VALUES (?1, ?2, ?3)",
                    params![v.page.to_string(), v.ts as i64, json],
                );
            }
        }
        let _ = tx.commit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut s = SqliteStore::open_in_memory().unwrap();
        assert!(s.load().is_none(), "fresh store has no data");
        let ws = Workspace::with_welcome();
        for p in &ws.pages {
            s.save_page(p);
        }
        s.save_settings(&Settings::default());
        let (loaded, _) = s.load().unwrap();
        assert_eq!(loaded.pages.len(), ws.pages.len());
        assert_eq!(loaded.pages[0].id, ws.pages[0].id, "order is preserved");
        s.remove_page(ws.pages[0].id);
        assert_eq!(s.load().unwrap().0.pages.len(), ws.pages.len() - 1);
    }
}
