//! replor の TUI。起動の引数・設定ファイル・Files lens と、mdvu から取り込んだ
//! pager を持つ（`docs/design/p1-core.md` の「crate の構成」）。
//!
//! binary は `main.rs` だけで、ここは binary の中身を tests から呼べるように分けたもの。

pub mod app;
pub mod config;
pub mod pager;
pub mod preview;
pub mod start;
pub mod tree;
