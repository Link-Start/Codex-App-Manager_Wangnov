//! codex-mac-engine
//!
//! Pure logic for planning Codex macOS updates from a Sparkle appcast.
//! Deliberately free of any Tauri dependency so it compiles and tests fast
//! in isolation. The Tauri backend depends on this crate via a path dependency
//! and supplies the GUI / command surface.
//!
//! Scope of this slice (read-only, safe):
//!   - parse the Sparkle appcast (versions + full enclosure + binary deltas)
//!   - given the installed build number, compute an UpdatePlan (delta vs full)
//!
//! Out of scope here (later, destructive — guarded elsewhere):
//!   - download, EdDSA verify, BinaryDelta apply, atomic swap, relaunch

pub mod appcast;
pub mod plan;
pub mod sys;

pub use appcast::{parse_appcast, Appcast, AppcastItem, Delta, Enclosure};
pub use plan::{plan_update, UpdatePlan, UpdateStrategy};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("failed to parse appcast: {0}")]
    Parse(String),
    #[error("appcast contained no usable items")]
    EmptyAppcast,
    #[error("io error: {0}")]
    Io(String),
}
