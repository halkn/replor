//! mdvu's pager, without its event loop: `crate::app` drives it as the right
//! pane. The terminal guard lives here.

pub mod event;
pub mod help;
pub mod images;
pub mod search;
mod search_session;
pub mod state;
pub mod view;

use std::io::{Stdout, Write, stdout};

use crossterm::cursor::{Hide, Show};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

/// Owns every terminal mode change the pager makes and undoes them on drop, so
/// a normal exit, an error and a panic all leave the terminal usable.
pub struct TerminalGuard {
    active: bool,
    restore: fn(),
}

impl TerminalGuard {
    pub fn enter() -> std::io::Result<Self> {
        install_panic_hook();
        enable_raw_mode()?;
        Self::finish_enter(
            || {
                let mut out = stdout();
                execute!(out, EnterAlternateScreen, Hide)?;
                out.flush()
            },
            restore,
        )
    }

    fn finish_enter(
        initialize: impl FnOnce() -> std::io::Result<()>,
        restore: fn(),
    ) -> std::io::Result<Self> {
        let guard = Self {
            active: true,
            restore,
        };
        initialize()?;
        Ok(guard)
    }

    pub fn leave(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        (self.restore)();
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.leave();
    }
}

fn restore() {
    let mut out: Stdout = stdout();
    let _ = leave_screen(&mut out);
    let _ = disable_raw_mode();
    let _ = out.flush();
}

/// Images go first: kitty stores them per screen, so once the main screen is
/// back a free would miss them and could hit the main screen's own.
fn leave_screen(out: &mut impl Write) -> std::io::Result<()> {
    out.write_all(images::take_release().as_bytes())?;
    execute!(out, LeaveAlternateScreen, Show)
}

/// Restore the terminal before the default panic handler prints, then let the
/// panic continue as usual.
fn install_panic_hook() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // A diagram's panic is caught and shown as that diagram's error,
            // and the TUI goes on, so neither the terminal nor the screen may
            // be touched.
            if replor_render::diagram::mermaid::panic_is_caught() {
                return;
            }
            restore();
            previous(info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn images_are_freed_before_leaving_the_alternate_screen() {
        use replor_render::image::{CellSize, ImageAsset, ImageSupport, Protocol, protocol};
        let bytes = include_bytes!(
            "../../../replor-render/tests/fixtures/gfm/.attachments/architecture.png"
        );
        let asset = ImageAsset::from_png(std::rc::Rc::from(bytes.as_slice()), 400).unwrap();
        let support = ImageSupport {
            protocol: Protocol::Kitty,
            cell: CellSize {
                width: 10,
                height: 20,
            },
        };
        let image = asset.place(support, 80).unwrap();
        let id = image.id;
        let line = replor_render::layout::RenderedLine {
            image: Some(image),
            ..Default::default()
        };
        let mut session = images::ImageSession::with_writer(Some(Protocol::Kitty), Vec::new());
        session
            .draw(images::visible(&[line], 0, 0, 80, 24))
            .unwrap();
        let mut out = Vec::new();
        leave_screen(&mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        let free = protocol::free(id);
        assert!(out.starts_with(&free), "{out:?}");
        assert!(out[free.len()..].contains("\x1b[?1049l"));
    }

    #[test]
    fn restoration_runs_once_on_partial_failure_normal_exit_and_panic() {
        static RESTORED: AtomicUsize = AtomicUsize::new(0);
        fn restore_test() {
            RESTORED.fetch_add(1, Ordering::SeqCst);
        }
        let result = TerminalGuard::finish_enter(
            || Err(std::io::Error::other("screen failed")),
            restore_test,
        );
        assert!(result.is_err());
        assert_eq!(RESTORED.load(Ordering::SeqCst), 1);
        {
            let mut guard = TerminalGuard::finish_enter(|| Ok(()), restore_test).unwrap();
            guard.leave();
            guard.leave();
        }
        assert_eq!(RESTORED.load(Ordering::SeqCst), 2);
        let _ = std::panic::catch_unwind(|| {
            let _guard = TerminalGuard::finish_enter(|| Ok(()), restore_test).unwrap();
            panic!("draw failed");
        });
        assert_eq!(RESTORED.load(Ordering::SeqCst), 3);
    }
}
