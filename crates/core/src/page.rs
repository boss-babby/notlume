//! Pages, the page tree and workspace-level settings.

use crate::block::{Block, BlockKind};
use crate::database::Database;
use crate::richtext::{MarkKind, RichText};
use crate::time::now_ms;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Cover {
    Color([u8; 3]),
    Url(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    pub id: Uuid,
    pub title: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub cover: Option<Cover>,
    #[serde(default)]
    pub parent: Option<Uuid>,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub blocks: Vec<Block>,
    #[serde(default)]
    pub database: Option<Database>,
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub updated: u64,
    /// Deleted top-level block ids -> deletion time (for sync).
    #[serde(default)]
    pub tombstones: HashMap<Uuid, u64>,
}

impl Page {
    pub fn new(title: &str, parent: Option<Uuid>) -> Self {
        let now = now_ms();
        Self {
            id: Uuid::new_v4(),
            title: title.to_owned(),
            icon: "📄".to_owned(),
            cover: None,
            parent,
            favorite: false,
            blocks: vec![Block::new(BlockKind::Paragraph)],
            database: None,
            created: now,
            updated: now,
            tombstones: HashMap::new(),
        }
    }

    pub fn touch(&mut self) {
        self.updated = now_ms();
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Version {
    pub page: Uuid,
    pub ts: u64,
    pub title: String,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Workspace {
    pub pages: Vec<Page>,
    #[serde(default)]
    pub versions: Vec<Version>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    Light,
    Dark,
    HighContrast,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub theme: ThemeMode,
    pub accent: [u8; 3],
    pub reduce_motion: bool,
    pub sidebar_open: bool,
    pub last_page: Option<Uuid>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeMode::Light,
            accent: [35, 131, 226],
            reduce_motion: false,
            sidebar_open: true,
            last_page: None,
        }
    }
}

const MAX_VERSIONS_PER_PAGE: usize = 20;

impl Workspace {
    /// A workspace with a short "Getting started" tour.
    pub fn with_welcome() -> Self {
        let mut ws = Self::default();
        let mut home = Page::new("Getting started", None);
        home.icon = "👋".into();
        home.cover = Some(Cover::Color([250, 222, 201]));
        let mut mention = Block::with_text(BlockKind::Paragraph, "Type / for commands, @ to link pages, and drag ⠿ to reorder.");
        mention.text.toggle(0, 6, MarkKind::Bold);
        let mut callout = Block::with_text(BlockKind::Callout, "Everything autosaves locally. Sign in to sync across devices.");
        callout.emoji = "💡".into();
        let mut code = Block::with_text(BlockKind::Code, "fn main() {\n    println!(\"hello, workspace\");\n}");
        code.language = "rust".into();
        let mut toggle = Block::with_text(BlockKind::Toggle, "Toggle blocks hide detail");
        toggle.children.push(Block::with_text(BlockKind::Paragraph, "Like this."));
        home.blocks = vec![
            Block::with_text(BlockKind::Heading(1), "Welcome"),
            mention,
            callout,
            Block::with_text(BlockKind::Heading(2), "Try it"),
            Block::with_text(BlockKind::Todo, "Press Ctrl+K to open the command palette"),
            Block::with_text(BlockKind::Todo, "Type '# ' at the start of a line to make a heading"),
            Block::with_text(BlockKind::Bulleted, "Bulleted item"),
            Block::with_text(BlockKind::Numbered, "Numbered item"),
            toggle,
            Block::with_text(BlockKind::Quote, "Simplicity is the ultimate sophistication."),
            code,
            Block::new(BlockKind::Divider),
            Block::new(BlockKind::Paragraph),
        ];
        let home_id = home.id;
        ws.pages.push(home);
        let mut tasks = Page::new("Tasks", None);
        tasks.icon = "✅".into();
        tasks.blocks.clear();
        tasks.database = Some(Database::new_tasks());
        ws.pages.push(tasks);
        let mut child = Page::new("Nested page", Some(home_id));
        child.icon = "🌱".into();
        ws.pages.push(child);
        ws
    }

    pub fn index_of(&self, id: Uuid) -> Option<usize> {
        self.pages.iter().position(|p| p.id == id)
    }

    pub fn page(&self, id: Uuid) -> Option<&Page> {
        self.pages.iter().find(|p| p.id == id)
    }

    pub fn page_mut(&mut self, id: Uuid) -> Option<&mut Page> {
        self.pages.iter_mut().find(|p| p.id == id)
    }

    pub fn add_page(&mut self, parent: Option<Uuid>) -> Uuid {
        let p = Page::new("Untitled", parent);
        let id = p.id;
        self.pages.push(p);
        id
    }

    pub fn add_database(&mut self, parent: Option<Uuid>) -> Uuid {
        let mut p = Page::new("Untitled database", parent);
        p.icon = "🗂".into();
        p.blocks.clear();
        p.database = Some(Database::new_tasks());
        let id = p.id;
        self.pages.push(p);
        id
    }

    pub fn children(&self, parent: Option<Uuid>) -> Vec<Uuid> {
        self.pages.iter().filter(|p| p.parent == parent).map(|p| p.id).collect()
    }

    pub fn is_descendant(&self, id: Uuid, ancestor: Uuid) -> bool {
        let mut cur = self.page(id).and_then(|p| p.parent);
        let mut guard = 0;
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = self.page(c).and_then(|p| p.parent);
            guard += 1;
            if guard > 10_000 {
                break;
            }
        }
        false
    }

    /// Re-parent `id` under `new_parent`, placed before sibling `before`
    /// (or last). Refuses to create cycles.
    pub fn move_page(&mut self, id: Uuid, new_parent: Option<Uuid>, before: Option<Uuid>) -> bool {
        if Some(id) == new_parent || Some(id) == before {
            return false;
        }
        if let Some(np) = new_parent {
            if self.is_descendant(np, id) {
                return false;
            }
        }
        let Some(from) = self.index_of(id) else { return false };
        let mut page = self.pages.remove(from);
        page.parent = new_parent;
        page.touch();
        let at = before.and_then(|b| self.index_of(b)).unwrap_or(self.pages.len());
        self.pages.insert(at, page);
        true
    }

    /// Remove a page and its descendants; returns every removed id.
    pub fn remove_page(&mut self, id: Uuid) -> Vec<Uuid> {
        let mut removed = vec![id];
        let mut i = 0;
        while i < removed.len() {
            let cur = removed[i];
            removed.extend(self.children(Some(cur)));
            i += 1;
        }
        self.pages.retain(|p| !removed.contains(&p.id));
        self.versions.retain(|v| !removed.contains(&v.page));
        removed
    }

    pub fn breadcrumb(&self, id: Uuid) -> Vec<(Uuid, String)> {
        let mut out = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            let Some(p) = self.page(c) else { break };
            out.push((p.id, format!("{} {}", p.icon, p.title)));
            cur = p.parent;
            if out.len() > 64 {
                break;
            }
        }
        out.reverse();
        out
    }

    pub fn snapshot_version(&mut self, id: Uuid) {
        let Some(p) = self.page(id) else { return };
        let v = Version {
            page: id,
            ts: now_ms(),
            title: p.title.clone(),
            blocks: p.blocks.clone(),
        };
        if let Some(last) = self.versions.iter().rev().find(|x| x.page == id) {
            if last.blocks == v.blocks && last.title == v.title {
                return;
            }
        }
        self.versions.push(v);
        let count = self.versions.iter().filter(|x| x.page == id).count();
        if count > MAX_VERSIONS_PER_PAGE {
            if let Some(pos) = self.versions.iter().position(|x| x.page == id) {
                self.versions.remove(pos);
            }
        }
    }

    pub fn versions_of(&self, id: Uuid) -> Vec<&Version> {
        self.versions.iter().filter(|v| v.page == id).rev().collect()
    }

    pub fn restore_version(&mut self, id: Uuid, ts: u64) -> bool {
        self.snapshot_version(id);
        let Some(v) = self.versions.iter().find(|v| v.page == id && v.ts == ts).cloned() else {
            return false;
        };
        if let Some(p) = self.page_mut(id) {
            p.blocks = v.blocks;
            p.title = v.title;
            p.touch();
            return true;
        }
        false
    }

    pub fn import_markdown(&mut self, title: &str, md: &str, parent: Option<Uuid>) -> Uuid {
        let mut p = Page::new(title, parent);
        p.blocks = crate::markdown::markdown_to_blocks(md);
        if p.blocks.is_empty() {
            p.blocks.push(Block::new(BlockKind::Paragraph));
        }
        let id = p.id;
        self.pages.push(p);
        id
    }
}

/// Convenience for building mention links.
pub fn mention_text(title: &str, target: Uuid) -> RichText {
    let mut t = RichText::new(&format!("@{title}"));
    let n = t.char_len();
    t.toggle(0, n, MarkKind::Link(format!("page:{target}")));
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_moves_and_cycles() {
        let mut ws = Workspace::default();
        let a = ws.add_page(None);
        let b = ws.add_page(Some(a));
        let c = ws.add_page(Some(b));
        assert!(ws.is_descendant(c, a));
        assert!(!ws.move_page(a, Some(c), None), "cycle must be refused");
        assert!(ws.move_page(c, None, Some(a)));
        assert_eq!(ws.page(c).unwrap().parent, None);
        assert_eq!(ws.pages[0].id, c);
    }

    #[test]
    fn remove_cascades_and_versions_dedupe() {
        let mut ws = Workspace::with_welcome();
        let home = ws.pages[0].id;
        ws.snapshot_version(home);
        ws.snapshot_version(home);
        assert_eq!(ws.versions_of(home).len(), 1);
        let removed = ws.remove_page(home);
        assert_eq!(removed.len(), 2);
        assert!(ws.versions.is_empty());
    }

    #[test]
    fn breadcrumb_order() {
        let mut ws = Workspace::default();
        let a = ws.add_page(None);
        let b = ws.add_page(Some(a));
        let bc = ws.breadcrumb(b);
        assert_eq!(bc.len(), 2);
        assert_eq!(bc[0].0, a);
    }
}
