//! S3: 実在する PR の unified diff を、最小の Ratatui アプリで unified / split に描く。
//!
//! `bench` は TestBackend で描画時間を測る。`tui` は実端末で体感を確かめる。
//! どちらも「全行を組み立てて Paragraph の scroll で切り出す（full）」と
//! 「見えている行だけを組み立てる（window）」の2方式を持つ。

mod diff;
mod highlight;

use std::io::Read;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::{Backend, TestBackend};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use diff::{FileDiff, Kind, Pair};
use highlight::Spans;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Unified,
    Split,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Strategy {
    Full,
    Window,
}

struct Prepared {
    file: FileDiff,
    pairs: Vec<Pair>,
    spans: Option<Vec<Spans>>,
}

struct App {
    files: Vec<Prepared>,
    current: usize,
    scroll: usize,
    mode: Mode,
    strategy: Strategy,
    last_frame: Duration,
    last_highlight: Duration,
    viewport: usize,
}

impl App {
    fn new(files: Vec<FileDiff>) -> Self {
        let files = files
            .into_iter()
            .map(|file| Prepared {
                pairs: diff::pairs(&file.rows),
                file,
                spans: None,
            })
            .collect();
        Self {
            files,
            current: 0,
            scroll: 0,
            mode: Mode::Unified,
            strategy: Strategy::Window,
            last_frame: Duration::ZERO,
            last_highlight: Duration::ZERO,
            viewport: 0,
        }
    }

    /// ハイライトは初めて開いたときに 1 回だけ行い、以後は使い回す。
    fn ensure_highlight(&mut self) {
        let p = &mut self.files[self.current];
        if p.spans.is_none() {
            let start = Instant::now();
            p.spans = Some(highlight::highlight_file(&p.file));
            self.last_highlight = start.elapsed();
        }
    }

    fn len(&self) -> usize {
        let p = &self.files[self.current];
        match self.mode {
            Mode::Unified => p.file.rows.len(),
            Mode::Split => p.pairs.len(),
        }
    }

    fn max_scroll(&self) -> usize {
        self.len().saturating_sub(self.viewport.max(1))
    }

    fn select(&mut self, index: usize) {
        self.current = index.min(self.files.len() - 1);
        self.scroll = 0;
        self.ensure_highlight();
    }

    fn draw(&mut self, frame: &mut ratatui::Frame) {
        let [body, status] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());
        let [list, main] =
            Layout::horizontal([Constraint::Length(32), Constraint::Min(10)]).areas(body);
        self.viewport = main.height.saturating_sub(2) as usize;
        self.draw_list(frame, list);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(self.files[self.current].file.path.as_str());
        let inner = block.inner(main);
        frame.render_widget(block, main);
        match self.mode {
            Mode::Unified => self.draw_unified(frame, inner),
            Mode::Split => self.draw_split(frame, inner),
        }
        let text = format!(
            " {}/{} | {:?} {:?} | 行 {}/{} | 描画 {:.2} ms | ハイライト {:.1} ms | j/k PgDn/PgUp g/G n/p:ファイル s:split w:方式 q:終了",
            self.current + 1,
            self.files.len(),
            self.mode,
            self.strategy,
            self.scroll + 1,
            self.len(),
            self.last_frame.as_secs_f64() * 1000.0,
            self.last_highlight.as_secs_f64() * 1000.0,
        );
        frame.render_widget(
            Paragraph::new(text).style(Style::default().add_modifier(Modifier::REVERSED)),
            status,
        );
    }

    fn draw_list(&self, frame: &mut ratatui::Frame, area: Rect) {
        let height = area.height.saturating_sub(2) as usize;
        let top = self.current.saturating_sub(height / 2);
        let lines: Vec<Line> = self
            .files
            .iter()
            .enumerate()
            .skip(top)
            .take(height)
            .map(|(i, p)| {
                let name = p.file.path.rsplit('/').next().unwrap_or(&p.file.path);
                let text = format!("{:>5} {name}", p.file.changed_lines());
                if i == self.current {
                    Line::styled(text, Style::default().add_modifier(Modifier::REVERSED))
                } else {
                    Line::raw(text)
                }
            })
            .collect();
        frame.render_widget(
            Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("files")),
            area,
        );
    }

    fn visible(&self, total: usize, height: usize) -> std::ops::Range<usize> {
        match self.strategy {
            Strategy::Full => 0..total,
            Strategy::Window => self.scroll.min(total)..(self.scroll + height).min(total),
        }
    }

    fn scroll_offset(&self) -> u16 {
        match self.strategy {
            Strategy::Full => self.scroll as u16,
            Strategy::Window => 0,
        }
    }

    fn draw_unified(&self, frame: &mut ratatui::Frame, area: Rect) {
        let p = &self.files[self.current];
        let spans = p.spans.as_ref().expect("highlighted");
        let range = self.visible(p.file.rows.len(), area.height as usize);
        let lines: Vec<Line> = range
            .map(|i| {
                let row = &p.file.rows[i];
                let gutter = format!(
                    "{:>5} {:>5} {} ",
                    row.old_no.map_or(String::new(), |n| n.to_string()),
                    row.new_no.map_or(String::new(), |n| n.to_string()),
                    sign(row.kind)
                );
                line(gutter, &spans[i], background(row.kind))
            })
            .collect();
        frame.render_widget(
            Paragraph::new(lines).scroll((self.scroll_offset(), 0)),
            area,
        );
    }

    fn draw_split(&self, frame: &mut ratatui::Frame, area: Rect) {
        let p = &self.files[self.current];
        let spans = p.spans.as_ref().expect("highlighted");
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(area);
        let range = self.visible(p.pairs.len(), area.height as usize);
        let mut olds = Vec::with_capacity(range.len());
        let mut news = Vec::with_capacity(range.len());
        for pair in &p.pairs[range] {
            if let Some(h) = pair.hunk {
                let l = line(String::new(), &spans[h], None);
                olds.push(l.clone());
                news.push(l);
                continue;
            }
            let half = |index: Option<usize>, old: bool| match index {
                Some(i) => {
                    let row = &p.file.rows[i];
                    let no = if old { row.old_no } else { row.new_no };
                    let kind = if row.kind == Kind::Context {
                        Kind::Context
                    } else {
                        row.kind
                    };
                    line(
                        format!("{:>5} ", no.unwrap_or(0)),
                        &spans[i],
                        background(kind),
                    )
                }
                None => Line::styled("", Style::default().bg(Color::Rgb(40, 40, 40))),
            };
            olds.push(half(pair.old, true));
            news.push(half(pair.new, false));
        }
        let offset = (self.scroll_offset(), 0);
        frame.render_widget(Paragraph::new(olds).scroll(offset), left);
        frame.render_widget(Paragraph::new(news).scroll(offset), right);
    }
}

fn sign(kind: Kind) -> char {
    match kind {
        Kind::Deletion => '-',
        Kind::Insertion => '+',
        _ => ' ',
    }
}

fn background(kind: Kind) -> Option<Color> {
    match kind {
        Kind::Deletion => Some(Color::Rgb(70, 25, 25)),
        Kind::Insertion => Some(Color::Rgb(25, 60, 25)),
        _ => None,
    }
}

fn line(gutter: String, spans: &Spans, bg: Option<Color>) -> Line<'static> {
    let base = bg.map_or(Style::default(), |c| Style::default().bg(c));
    let mut out = vec![Span::styled(gutter, base.fg(Color::DarkGray))];
    for (text, fg) in spans {
        let style = fg.map_or(base, |c| base.fg(c));
        out.push(Span::styled(text.clone(), style));
    }
    Line::from(out).style(base)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("");
    let path = args.get(1).map(String::as_str).unwrap_or("-");
    let mut text = String::new();
    if path == "-" {
        std::io::stdin().read_to_string(&mut text).unwrap();
    } else {
        text = String::from_utf8_lossy(&std::fs::read(path).expect("read diff")).into_owned();
    }
    let parse_start = Instant::now();
    let files = diff::parse(&text);
    let parse_time = parse_start.elapsed();
    match command {
        "bench" => bench(files, parse_time),
        "tui" => tui(files).unwrap(),
        _ => {
            eprintln!("usage: s3-diff bench|tui <diff-file|->");
            std::process::exit(2);
        }
    }
}

fn bench(files: Vec<FileDiff>, parse_time: Duration) {
    let rows: usize = files.iter().map(|f| f.rows.len()).sum();
    let changed: usize = files.iter().map(FileDiff::changed_lines).sum();
    let biggest = files.iter().map(|f| f.rows.len()).max().unwrap_or(0);
    println!(
        "files={} rows={rows} changed={changed} biggest_file_rows={biggest} parse={:.1} ms",
        files.len(),
        parse_time.as_secs_f64() * 1000.0
    );
    let mut app = App::new(files);

    // ハイライトの費用（ファイルを初めて開くときの待ち）。
    let mut highlight_times = Vec::new();
    for i in 0..app.files.len() {
        app.select(i);
        highlight_times.push(app.last_highlight);
    }
    report("highlight per file (初回オープン)", &mut highlight_times);

    let mut terminal = Terminal::new(TestBackend::new(200, 50)).unwrap();
    for mode in [Mode::Unified, Mode::Split] {
        for strategy in [Strategy::Full, Strategy::Window] {
            app.mode = mode;
            app.strategy = strategy;
            // 全ファイルを、1 行ずつ 100 回と、1 画面ずつ末尾まで送る。
            let mut frames = Vec::new();
            let mut biggest_frames = Vec::new();
            let biggest_index = (0..app.files.len())
                .max_by_key(|&i| app.files[i].file.rows.len())
                .unwrap();
            for i in 0..app.files.len() {
                app.select(i);
                let mut times = Vec::new();
                draw(&mut terminal, &mut app, &mut times);
                for _ in 0..100 {
                    app.scroll = (app.scroll + 1).min(app.max_scroll());
                    draw(&mut terminal, &mut app, &mut times);
                }
                while app.scroll < app.max_scroll() {
                    app.scroll = (app.scroll + app.viewport).min(app.max_scroll());
                    draw(&mut terminal, &mut app, &mut times);
                }
                if i == biggest_index {
                    biggest_frames = times.clone();
                }
                frames.extend(times);
            }
            report(&format!("{mode:?} {strategy:?} 全ファイル"), &mut frames);
            report(
                &format!("{mode:?} {strategy:?} 最大のファイル"),
                &mut biggest_frames,
            );
        }
    }
}

fn draw<B: Backend>(terminal: &mut Terminal<B>, app: &mut App, times: &mut Vec<Duration>) {
    let start = Instant::now();
    terminal.draw(|f| app.draw(f)).unwrap();
    let elapsed = start.elapsed();
    app.last_frame = elapsed;
    times.push(elapsed);
}

fn report(name: &str, times: &mut [Duration]) {
    if times.is_empty() {
        return;
    }
    times.sort();
    let pick = |q: f64| times[((times.len() - 1) as f64 * q) as usize].as_secs_f64() * 1000.0;
    println!(
        "{name:<36} n={:>6}  p50 {:>8.3} ms  p95 {:>8.3} ms  max {:>8.3} ms",
        times.len(),
        pick(0.5),
        pick(0.95),
        pick(1.0)
    );
}

fn tui(files: Vec<FileDiff>) -> std::io::Result<()> {
    let mut app = App::new(files);
    app.select(0);
    let mut terminal = ratatui::init();
    let result = (|| -> std::io::Result<()> {
        loop {
            let start = Instant::now();
            terminal.draw(|f| app.draw(f))?;
            app.last_frame = start.elapsed();
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            let page = app.viewport.max(1);
            match key.code {
                KeyCode::Char('q') => return Ok(()),
                KeyCode::Char('j') | KeyCode::Down => app.scroll += 1,
                KeyCode::Char('k') | KeyCode::Up => app.scroll = app.scroll.saturating_sub(1),
                KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.scroll += page / 2
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.scroll = app.scroll.saturating_sub(page / 2)
                }
                KeyCode::PageDown | KeyCode::Char(' ') => app.scroll += page,
                KeyCode::PageUp => app.scroll = app.scroll.saturating_sub(page),
                KeyCode::Char('g') => app.scroll = 0,
                KeyCode::Char('G') => app.scroll = usize::MAX,
                KeyCode::Char('n') | KeyCode::Char(']') => app.select(app.current + 1),
                KeyCode::Char('p') | KeyCode::Char('[') => {
                    app.select(app.current.saturating_sub(1))
                }
                KeyCode::Char('s') => {
                    app.mode = match app.mode {
                        Mode::Unified => Mode::Split,
                        Mode::Split => Mode::Unified,
                    };
                    app.scroll = 0;
                }
                KeyCode::Char('w') => {
                    app.strategy = match app.strategy {
                        Strategy::Full => Strategy::Window,
                        Strategy::Window => Strategy::Full,
                    }
                }
                _ => {}
            }
            app.scroll = app.scroll.min(app.max_scroll());
        }
    })();
    ratatui::restore();
    result
}
