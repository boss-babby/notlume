//! Shared, UI-free core of the workspace app.
//!
//! Everything here compiles for native and `wasm32-unknown-unknown`.

pub mod block;
pub mod database;
pub mod export;
pub mod history;
pub mod markdown;
pub mod page;
pub mod richtext;
pub mod search;
pub mod sync;
pub mod time;

pub use block::{Block, BlockKind};
pub use database::Database;
pub use history::History;
pub use page::{Cover, Page, Settings, ThemeMode, Workspace};
pub use richtext::{Mark, MarkKind, RichText};
pub use uuid::Uuid;
