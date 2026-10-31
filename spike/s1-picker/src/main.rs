//! S1 の比較用: docsail と同じく ratatui-image の `Picker::from_query_stdio` で protocol を
//! 選び、同じ PNG を表示する。mdvu の方式で画像が出ない環境があったときだけ使う。
//!
//! 使い方: s1-picker <png>...   ←/→ で画像を切り替え、q で終了。
//! 起動時の問い合わせにかかった時間と、選ばれた protocol を画面の下端に出す。

use std::time::Instant;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui_image::StatefulImage;
use ratatui_image::picker::Picker;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: s1-picker <png>...");
        std::process::exit(2);
    }
    let images = paths
        .iter()
        .map(|p| image::ImageReader::open(p)?.with_guessed_format()?.decode())
        .collect::<Result<Vec<_>, _>>()?;

    let mut terminal = ratatui::init();
    // 問い合わせは alternate screen と raw mode に入ってから行う（docsail と同じ順序）。
    let start = Instant::now();
    let picker = Picker::from_query_stdio();
    let query = start.elapsed();
    let (picker, note) = match picker {
        Ok(p) => (p, String::new()),
        Err(e) => (Picker::halfblocks(), format!(" 問い合わせ失敗: {e}")),
    };
    let protocol = picker.protocol_type();
    let font = picker.font_size();
    let mut current = 0;
    let mut state = picker.new_resize_protocol(images[current].clone());

    let result = (|| -> std::io::Result<()> {
        loop {
            terminal.draw(|f| {
                let [body, status] =
                    Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(f.area());
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(paths[current].as_str());
                let inner = block.inner(body);
                f.render_widget(block, body);
                f.render_stateful_widget(StatefulImage::default(), inner, &mut state);
                f.render_widget(
                    Paragraph::new(format!(
                        " protocol={protocol:?} font={}x{} 問い合わせ {:.1} ms{note} | ←/→ 画像 q 終了",
                        font.width,
                        font.height,
                        query.as_secs_f64() * 1000.0
                    )),
                    status,
                );
            })?;
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') => return Ok(()),
                KeyCode::Right | KeyCode::Char('l') => {
                    current = (current + 1) % images.len();
                    state = picker.new_resize_protocol(images[current].clone());
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    current = (current + images.len() - 1) % images.len();
                    state = picker.new_resize_protocol(images[current].clone());
                }
                _ => {}
            }
        }
    })();
    ratatui::restore();
    result?;
    Ok(())
}
