#![allow(clippy::too_many_lines)]

use bitcoin::{Transaction, consensus::Decodable};
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use std::{
    io,
    path::PathBuf,
    time::{Duration, Instant},
};
use windfish::{MempoolSerde, Txn};

#[derive(Parser)]
#[command(name = "windfish-tui")]
#[command(about = "TUI editor for Bitcoin mempool.dat files", long_about = None)]
struct Args {
    /// Input mempool.dat file path
    #[arg(short, long)]
    input: PathBuf,

    /// Output mempool.dat file path
    #[arg(short, long)]
    output: PathBuf,
}

struct App {
    mempool: MempoolSerde,
    list_state: ListState,
    output_path: PathBuf,
    mode: Mode,
    input_buffer: String,
    status_message: Option<(String, Instant)>,
    animation_tick: u64,
}

#[derive(PartialEq, Eq)]
enum Mode {
    Normal,
    Insert,
}

impl App {
    fn new(mempool: MempoolSerde, output_path: PathBuf) -> Self {
        let mut list_state = ListState::default();
        if !mempool.txs.is_empty() {
            list_state.select(Some(0));
        }
        Self {
            mempool,
            list_state,
            output_path,
            mode: Mode::Normal,
            input_buffer: String::new(),
            status_message: None,
            animation_tick: 0,
        }
    }

    fn selected_tx(&self) -> Option<&Txn> {
        self.list_state
            .selected()
            .and_then(|i| self.mempool.txs.get(i))
    }

    fn next(&mut self) {
        let len = self.mempool.txs.len();
        if len == 0 {
            return;
        }
        let i = self.list_state.selected().map_or(0, |i| (i + 1) % len);
        self.list_state.select(Some(i));
    }

    fn previous(&mut self) {
        let len = self.mempool.txs.len();
        if len == 0 {
            return;
        }
        let i = self
            .list_state
            .selected()
            .map_or(0, |i| if i == 0 { len - 1 } else { i - 1 });
        self.list_state.select(Some(i));
    }

    fn delete_selected(&mut self) {
        if let Some(i) = self.list_state.selected()
            && i < self.mempool.txs.len()
        {
            self.mempool.txs.remove(i);
            self.set_status("Transaction deleted".to_string());
            if self.mempool.txs.is_empty() {
                self.list_state.select(None);
            } else if i >= self.mempool.txs.len() {
                self.list_state.select(Some(self.mempool.txs.len() - 1));
            }
        }
    }

    fn insert_tx(&mut self, hex: &str) -> Result<(), String> {
        let bytes = hex::decode(hex.trim()).map_err(|e| format!("Invalid hex: {e}"))?;
        let tx: Transaction = Transaction::consensus_decode(&mut bytes.as_slice())
            .map_err(|e| format!("Invalid transaction: {e}"))?;

        let txn = Txn {
            tx,
            time: chrono::Utc::now().timestamp(),
            fee_delta: 0,
        };
        self.mempool.txs.push(txn);
        self.list_state.select(Some(self.mempool.txs.len() - 1));
        self.set_status("Transaction inserted".to_string());
        Ok(())
    }

    fn save(&self) -> Result<(), String> {
        self.mempool
            .write_to_file(&self.output_path)
            .map_err(|e| format!("Save failed: {e}"))
    }

    fn set_status(&mut self, msg: String) {
        self.status_message = Some((msg, Instant::now()));
    }

    fn tick(&mut self) {
        self.animation_tick = self.animation_tick.wrapping_add(1);
        if let Some((_, instant)) = &self.status_message
            && instant.elapsed() > Duration::from_secs(3)
        {
            self.status_message = None;
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let mempool = MempoolSerde::new(&args.input)?;
    let mut app = App::new(mempool, args.output);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let tick_rate = Duration::from_millis(50);
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if crossterm::event::poll(timeout)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match app.mode {
                Mode::Normal => match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Down | KeyCode::Char('j') => app.next(),
                    KeyCode::Up | KeyCode::Char('k') => app.previous(),
                    KeyCode::Char('d') => app.delete_selected(),
                    KeyCode::Char('i') => {
                        app.mode = Mode::Insert;
                        app.input_buffer.clear();
                    }
                    KeyCode::Char('s') => match app.save() {
                        Ok(()) => app.set_status("Saved successfully!".to_string()),
                        Err(e) => app.set_status(e),
                    },
                    _ => {}
                },
                Mode::Insert => match key.code {
                    KeyCode::Esc => {
                        app.mode = Mode::Normal;
                        app.input_buffer.clear();
                    }
                    KeyCode::Enter => {
                        let hex = app.input_buffer.clone();
                        match app.insert_tx(&hex) {
                            Ok(()) => app.mode = Mode::Normal,
                            Err(e) => app.set_status(e),
                        }
                        app.input_buffer.clear();
                    }
                    KeyCode::Backspace => {
                        app.input_buffer.pop();
                    }
                    KeyCode::Char(c) => app.input_buffer.push(c),
                    _ => {}
                },
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.tick();
            last_tick = Instant::now();
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

#[allow(clippy::cast_possible_truncation)]
fn ui(f: &mut Frame, app: &mut App) {
    let size = f.area();

    // Animated background effect
    let bg_color = Color::Rgb(0, (10 + (app.animation_tick % 20)) as u8, 0);

    let main_block = Block::default().style(Style::default().bg(bg_color));
    f.render_widget(main_block, size);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(size);

    // Header with animated title
    let glow = ((app.animation_tick % 30) as u8).saturating_mul(8);
    let title_style = Style::default()
        .fg(Color::Rgb(0, 255, glow.saturating_add(100)))
        .add_modifier(Modifier::BOLD);

    let header = Paragraph::new(Line::from(vec![
        Span::styled("◆ ", Style::default().fg(Color::Rgb(0, 255, 100))),
        Span::styled("WINDFISH", title_style),
        Span::styled(" ◆ ", Style::default().fg(Color::Rgb(0, 255, 100))),
        Span::styled("Mempool Editor", Style::default().fg(Color::DarkGray)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(0, 180, 0)))
            .style(Style::default().bg(Color::Rgb(0, 20, 0))),
    );
    f.render_widget(header, chunks[0]);

    // Main content area
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(chunks[1]);

    // Left panel - TX list
    let items: Vec<ListItem> = app
        .mempool
        .txs
        .iter()
        .enumerate()
        .map(|(i, txn)| {
            let txid = txn.tx.compute_txid().to_string();
            let short_txid = format!("{}...{}", &txid[..8], &txid[txid.len() - 8..]);

            let style = if Some(i) == app.list_state.selected() {
                Style::default()
                    .fg(Color::Rgb(0, 255, 0))
                    .bg(Color::Rgb(0, 50, 0))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(0, 200, 0))
            };

            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:3} ", i + 1),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(short_txid, style),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(Span::styled(
                    format!(" TXIDs ({}) ", app.mempool.txs.len()),
                    Style::default()
                        .fg(Color::Rgb(0, 255, 100))
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(0, 120, 0)))
                .style(Style::default().bg(Color::Rgb(0, 15, 0))),
        )
        .highlight_symbol("▶ ")
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    f.render_stateful_widget(list, content_chunks[0], &mut app.list_state);

    // Right panel - TX details
    let details = app.selected_tx().map_or_else(
        || {
            vec![Line::from(Span::styled(
                "No transaction selected",
                Style::default().fg(Color::DarkGray),
            ))]
        },
        |txn| {
            let txid = txn.tx.compute_txid();
            let datetime = chrono::DateTime::from_timestamp(txn.time, 0).map_or_else(
                || "Unknown".to_string(),
                |dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            );

            vec![
                Line::from(vec![
                    Span::styled("TXID: ", Style::default().fg(Color::Rgb(0, 150, 0))),
                    Span::styled(
                        txid.to_string(),
                        Style::default().fg(Color::Rgb(0, 255, 100)),
                    ),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Version: ", Style::default().fg(Color::Rgb(0, 150, 0))),
                    Span::styled(
                        txn.tx.version.to_string(),
                        Style::default().fg(Color::White),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Lock Time: ", Style::default().fg(Color::Rgb(0, 150, 0))),
                    Span::styled(
                        txn.tx.lock_time.to_string(),
                        Style::default().fg(Color::White),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Inputs: ", Style::default().fg(Color::Rgb(0, 150, 0))),
                    Span::styled(
                        txn.tx.input.len().to_string(),
                        Style::default().fg(Color::Cyan),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Outputs: ", Style::default().fg(Color::Rgb(0, 150, 0))),
                    Span::styled(
                        txn.tx.output.len().to_string(),
                        Style::default().fg(Color::Cyan),
                    ),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Time: ", Style::default().fg(Color::Rgb(0, 150, 0))),
                    Span::styled(datetime, Style::default().fg(Color::Yellow)),
                ]),
                Line::from(vec![
                    Span::styled("Fee Delta: ", Style::default().fg(Color::Rgb(0, 150, 0))),
                    Span::styled(
                        format!("{} sat", txn.fee_delta),
                        Style::default().fg(Color::Magenta),
                    ),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "─── Outputs ───",
                    Style::default().fg(Color::Rgb(0, 100, 0)),
                )),
            ]
            .into_iter()
            .chain(txn.tx.output.iter().enumerate().map(|(i, out)| {
                Line::from(vec![
                    Span::styled(format!("  [{i}] "), Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("{} sat", out.value.to_sat()),
                        Style::default().fg(Color::Rgb(255, 200, 0)),
                    ),
                ])
            }))
            .collect()
        },
    );

    let details_widget = Paragraph::new(details)
        .block(
            Block::default()
                .title(Span::styled(
                    " Details ",
                    Style::default()
                        .fg(Color::Rgb(0, 255, 100))
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(0, 120, 0)))
                .style(Style::default().bg(Color::Rgb(0, 15, 0))),
        )
        .wrap(Wrap { trim: true });

    f.render_widget(details_widget, content_chunks[1]);

    // Footer / Status bar
    let mode_indicator = match app.mode {
        Mode::Normal => Span::styled(
            " NORMAL ",
            Style::default().bg(Color::Rgb(0, 100, 0)).fg(Color::White),
        ),
        Mode::Insert => Span::styled(
            " INSERT ",
            Style::default()
                .bg(Color::Rgb(100, 100, 0))
                .fg(Color::Black),
        ),
    };

    let help_text = match app.mode {
        Mode::Normal => "q:quit  ↑↓/jk:nav  i:insert  d:delete  s:save",
        Mode::Insert => "Enter:confirm  Esc:cancel  (paste raw tx hex)",
    };

    let status = if let Some((msg, _)) = &app.status_message {
        Span::styled(
            format!(" {msg} "),
            Style::default().fg(Color::Rgb(255, 255, 0)),
        )
    } else {
        Span::styled("", Style::default())
    };

    let footer = Paragraph::new(Line::from(vec![
        mode_indicator,
        Span::raw(" "),
        Span::styled(help_text, Style::default().fg(Color::Rgb(0, 150, 0))),
        Span::raw("  "),
        status,
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(0, 80, 0)))
            .style(Style::default().bg(Color::Rgb(0, 10, 0))),
    );

    f.render_widget(footer, chunks[2]);

    // Insert mode popup
    if app.mode == Mode::Insert {
        let popup_area = centered_rect(70, 20, size);
        f.render_widget(Clear, popup_area);

        let input = Paragraph::new(app.input_buffer.as_str())
            .block(
                Block::default()
                    .title(Span::styled(
                        " Insert Raw Transaction (hex) ",
                        Style::default()
                            .fg(Color::Rgb(255, 255, 0))
                            .add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(200, 200, 0)))
                    .style(Style::default().bg(Color::Rgb(20, 20, 0))),
            )
            .wrap(Wrap { trim: false });

        f.render_widget(input, popup_area);
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
