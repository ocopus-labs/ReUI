//! Shared by the file moments (downloads, screenshots): watching folders without polling
//! and the shell calls their cards make. Like `audio/`, it holds no state of its own; each
//! module starts the watch it needs.
//!
//! Files: `notify.rs` (a change buffer as [`Change`]s; pure, tested), `watch.rs` (the
//! watching thread), `shell.rs` (known folders, Open, Show in folder, file icons).

mod notify;
pub mod shell;
mod watch;

pub use self::notify::Change;
pub use self::watch::Watch;
