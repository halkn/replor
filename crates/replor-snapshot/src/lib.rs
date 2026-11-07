//! Snapshot の実装: 作業ツリー（`WorktreeSnapshot`）と、gix で読むリビジョン
//! （`RevisionSnapshot`）。trait と、両者が守る契約は描画コアの
//! `replor_render::snapshot` と `docs/design/p1-core.md` の「Snapshot の trait」にある。

mod revision;
mod worktree;

pub use revision::RevisionSnapshot;
pub use worktree::WorktreeSnapshot;

/// Symlinks followed in one `resolve` before giving up, so a loop ends. The
/// working tree leaves the count to the OS; a revision counts here.
pub(crate) const MAX_SYMLINK_HOPS: usize = 40;
