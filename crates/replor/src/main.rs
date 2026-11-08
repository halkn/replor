use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use ratatui::layout::Rect;
use replor::app::{self, App};
use replor::config::{self, HighlightWhen, ImagesWhen, Settings};
use replor::preview::Options;
use replor::start;
use replor_render::diagram::MermaidMode;
use replor_render::image::{self, ImageSupport, Protocol, WindowSize};
use replor_render::layout::theme::{Theme, Variant};
use replor_render::output::ColorChoice;

/// Read a repository's Markdown, diagrams and images in the terminal.
#[derive(Debug, Parser)]
#[command(version)]
struct Cli {
    /// A directory to open the tree at, or a file to open. Defaults to `docs/`
    /// when the working directory has one.
    path: Option<PathBuf>,
}

fn main() {
    let cli = Cli::parse();
    if let Err(message) = run(cli) {
        eprintln!("replor: {}", replor_render::text::into_visible(message));
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let settings = config::load(|key| std::env::var(key).ok()).map_err(|err| err.to_string())?;
    let cwd = std::env::current_dir().map_err(|err| err.to_string())?;
    let start = start::resolve(cli.path.as_deref(), &cwd).map_err(|err| {
        let path = cli.path.as_deref().unwrap_or(&cwd);
        format!("{}: {err}", path.display())
    })?;
    let options = options(&settings);
    let theme = Theme::new(Variant::resolve(settings.theme)).with_color(ColorChoice::Ansi);
    let (columns, rows) = crossterm::terminal::size().map_err(|err| err.to_string())?;
    let mut app = App::new(
        Arc::new(start.snapshot),
        start.top,
        options,
        theme,
        Rect::new(0, 0, columns, rows),
    );
    if let Some(file) = &start.file {
        app.open(file);
    }
    if !settings.warnings.is_empty() {
        app.set_status(settings.warnings.join("; "));
    }
    app::run(app).map_err(|err| err.to_string())
}

fn options(settings: &Settings) -> Options {
    let protocol = match settings.images {
        ImagesWhen::Never => None,
        ImagesWhen::Kitty => Some(Protocol::Kitty),
        ImagesWhen::Iterm2 => Some(Protocol::Iterm2),
        ImagesWhen::Auto => image::protocol_from_env(&image::Env::detect()),
    };
    // The render core does not touch the terminal, so its size is read here.
    let window = crossterm::terminal::window_size()
        .ok()
        .map(|size| WindowSize {
            columns: size.columns,
            rows: size.rows,
            width: size.width,
            height: size.height,
        });
    let images = protocol.map(|protocol| ImageSupport::detect(protocol, window));
    // A picture needs a graphics protocol, so `image` becomes Unicode text
    // wherever images are off.
    let mermaid = match (settings.mermaid, images) {
        (MermaidMode::Image, None) => MermaidMode::Unicode,
        (mode, _) => mode,
    };
    Options {
        flavor: settings.flavor,
        mermaid,
        mermaid_width: settings.mermaid_width,
        highlight: settings.highlight == HighlightWhen::Auto,
        images,
        icons: settings.icons,
    }
}
