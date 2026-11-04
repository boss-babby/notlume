//! Shared egui UI: theme, animation, block editor, databases, sidebar and the
//! application shell. Platform crates only provide a [`storage::Persistence`]
//! implementation and call [`app::NotionApp::new`].

pub mod anim;
pub mod app;
pub mod db_view;
pub mod editor;
pub mod fonts;
pub mod storage;
pub mod sync_client;
pub mod syntax;
pub mod theme;

pub use app::{AppOptions, NotionApp};
pub use storage::{MemoryStore, Persistence};
