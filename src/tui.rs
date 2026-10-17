//! Interactive terminal view: the same servers, details and clean up as the
//! bar widget.

use crate::{
    actions, config,
    scan::{Scanner, Server, Snapshot, format_bytes, format_duration, home_relative},
};
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::{
        event::{
            self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
            KeyModifiers, MouseButton, MouseEventKind,
        },
        execute,
    },
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Sparkline, Table, TableState, Wrap},
};
use std::{
    collections::HashSet,
    io,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

const PALETTE: [(u8, u8, u8); 10] = [
    (0x7a, 0xa2, 0xf7),
    (0x9e, 0xce, 0x6a),
    (0xbb, 0x9a, 0xf7),
    (0x7d, 0xcf, 0xff),
    (0xe0, 0xaf, 0x68),
    (0xf7, 0x76, 0x8e),
    (0x73, 0xda, 0xca),
    (0xff, 0x9e, 0x64),
    (0x2a, 0xc3, 0xde),
    (0xc0, 0xca, 0xf5),
];
const WARNING: Color = Color::Rgb(0xe0, 0xaf, 0x68);

/// Matches Model.portColor, so a port has one colour everywhere.
fn port_color(port: u16) -> Color {
    let hash = (port as u64 * 2654435761) % (1 << 32) % PALETTE.len() as u64;
    let (r, g, b) = PALETTE[hash as usize];
    Color::Rgb(r, g, b)
}

#[derive(Clone, Copy, PartialEq)]
enum View {
    List,
    Details,
    Cleanup,
    Help,
}

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Stop,
    Restart,
    StopSelected,
}

enum Message {
    Snapshot(Box<Snapshot>),
    Done(String),
}

struct App {
    snapshot: Option<Snapshot>,
    table: TableState,
    view: View,
    selected: HashSet<String>,
    confirm: Option<(Action, String)>,
    status: Option<(String, Instant)>,
    grace: f64,
    tx: Sender<Message>,
    table_area: Rect,
    quit: bool,
}

impl App {
    fn servers(&self) -> &[Server] {
        self.snapshot
            .as_ref()
            .map(|s| s.servers.as_slice())
            .unwrap_or(&[])
    }

    fn current(&self) -> Option<&Server> {
        self.servers().get(self.table.selected().unwrap_or(0))
    }

    fn say(&mut self, text: impl Into<String>) {
        self.status = Some((text.into(), Instant::now()));
    }

    fn move_cursor(&mut self, delta: i64) {
        let count = self.servers().len() as i64;
        if count == 0 {
            return;
        }
        let next = (self.table.selected().unwrap_or(0) as i64 + delta).clamp(0, count - 1);
        self.table.select(Some(next as usize));
    }

    fn run_action(&mut self, action: Action, server_ids: Vec<String>) {
        let servers: Vec<Server> = self
            .servers()
            .iter()
            .filter(|s| server_ids.contains(&s.id))
            .cloned()
            .collect();
        if servers.is_empty() {
            return;
        }
        let tx = self.tx.clone();
        let grace = self.grace;
        self.say(match action {
            Action::Restart => "Restarting…".to_string(),
            _ => format!(
                "Stopping {} server{}…",
                servers.len(),
                if servers.len() == 1 { "" } else { "s" }
            ),
        });
        thread::spawn(move || {
            let results: Vec<String> = servers
                .iter()
                .map(|server| {
                    let name = actions::describe(server);
                    match action {
                        Action::Restart => match actions::restart(server, true, grace) {
                            Ok(log) => format!(
                                "Restarted {name} · output in {}",
                                home_relative(&log.display().to_string())
                            ),
                            Err(e) => format!("Cannot restart {name}: {e}"),
                        },
                        _ => match actions::stop(server, true, grace) {
                            Ok(_) => format!("Stopped {name}"),
                            Err(e) => format!("Cannot stop {name}: {e}"),
                        },
                    }
                })
                .collect();
            let _ = tx.send(Message::Done(results.join(" · ")));
        });
    }

    fn ask(&mut self, action: Action) {
        let id = match action {
            Action::StopSelected => String::new(),
            _ => match self.current() {
                Some(server) => server.id.clone(),
                None => return,
            },
        };
        if action == Action::StopSelected && self.selected.is_empty() {
            self.say("Nothing selected. Space selects a server.");
            return;
        }
        self.confirm = Some((action, id));
    }

    fn answer(&mut self, yes: bool) {
        let Some((action, id)) = self.confirm.take() else {
            return;
        };
        if !yes {
            self.say("Cancelled.");
            return;
        }
        if action == Action::StopSelected {
            let ids: Vec<String> = self.selected.drain().collect();
            self.view = View::List;
            self.run_action(Action::Stop, ids);
        } else {
            self.run_action(action, vec![id]);
        }
    }

    fn key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if self.confirm.is_some() {
            self.answer(matches!(
                code,
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter
            ));
            return;
        }
        if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
            return;
        }
        match (self.view, code) {
            (View::Help, _) => self.view = View::List,
            (_, KeyCode::Char('?')) => self.view = View::Help,
            (_, KeyCode::Up | KeyCode::Char('k')) => self.move_cursor(-1),
            (_, KeyCode::Down | KeyCode::Char('j')) => self.move_cursor(1),
            (_, KeyCode::PageUp) => self.move_cursor(-10),
            (_, KeyCode::PageDown) => self.move_cursor(10),
            (View::List, KeyCode::Char('q') | KeyCode::Esc) => self.quit = true,
            (View::List, KeyCode::Enter | KeyCode::Right | KeyCode::Char('l'))
                if self.current().is_some() =>
            {
                self.view = View::Details
            }
            (
                View::Details,
                KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('q'),
            ) => self.view = View::List,
            (View::List | View::Details, KeyCode::Char('c')) if !self.servers().is_empty() => {
                self.selected = self
                    .servers()
                    .iter()
                    .filter(|s| s.cleanup)
                    .map(|s| s.id.clone())
                    .collect();
                self.view = View::Cleanup;
            }
            (View::List | View::Details, KeyCode::Char('o')) => {
                if let Some(server) = self.current() {
                    let result = actions::open(server);
                    let url = server.url.clone();
                    self.say(match result {
                        Ok(()) => format!("Opened {url}"),
                        Err(e) => e,
                    });
                }
            }
            (View::List | View::Details, KeyCode::Char('s')) => self.ask(Action::Stop),
            (View::List | View::Details, KeyCode::Char('r')) => self.ask(Action::Restart),
            (View::Cleanup, KeyCode::Char(' ')) => {
                if let Some(server) = self.current().filter(|s| !s.protected) {
                    let id = server.id.clone();
                    if !self.selected.remove(&id) {
                        self.selected.insert(id);
                    }
                }
                self.move_cursor(1);
            }
            (View::Cleanup, KeyCode::Char('a')) => {
                self.selected = self
                    .servers()
                    .iter()
                    .filter(|s| !s.protected)
                    .map(|s| s.id.clone())
                    .collect();
            }
            (View::Cleanup, KeyCode::Char('n')) => self.selected.clear(),
            (View::Cleanup, KeyCode::Enter | KeyCode::Char('s')) => self.ask(Action::StopSelected),
            (View::Cleanup, KeyCode::Esc | KeyCode::Char('q')) => self.view = View::List,
            _ => {}
        }
    }

    fn click(&mut self, column: u16, row: u16) {
        let area = self.table_area;
        // One border row and one header row precede the first server.
        if column < area.x
            || column >= area.x + area.width
            || row < area.y + 2
            || row >= area.y + area.height
        {
            return;
        }
        let index = (row - area.y - 2) as usize + self.table.offset();
        if index >= self.servers().len() {
            return;
        }
        if self.table.selected() == Some(index) {
            match self.view {
                View::List => self.view = View::Details,
                View::Cleanup => self.key(KeyCode::Char(' '), KeyModifiers::NONE),
                _ => {}
            }
        } else {
            self.table.select(Some(index));
        }
    }
}

fn scanner_thread(mut watcher: config::Watcher, interval: f64, tx: Sender<Message>) {
    thread::spawn(move || {
        let mut scanner = Scanner::new(true);
        loop {
            watcher.refresh();
            let snapshot = scanner.scan(&watcher.settings);
            if tx.send(Message::Snapshot(Box::new(snapshot))).is_err() {
                return;
            }
            thread::sleep(Duration::from_secs_f64(interval));
        }
    });
}

pub fn run(watcher: config::Watcher, interval: f64) -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    let grace = watcher.settings.grace_seconds;
    scanner_thread(watcher, interval, tx.clone());
    let mut terminal = ratatui::init();
    let _ = execute!(io::stdout(), EnableMouseCapture);
    let mut app = App {
        snapshot: None,
        table: TableState::default().with_selected(Some(0)),
        view: View::List,
        selected: HashSet::new(),
        confirm: None,
        status: None,
        grace,
        tx,
        table_area: Rect::default(),
        quit: false,
    };
    let result = event_loop(&mut terminal, &mut app, &rx);
    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    rx: &Receiver<Message>,
) -> Result<(), String> {
    while !app.quit {
        while let Ok(message) = rx.try_recv() {
            match message {
                Message::Snapshot(snapshot) => {
                    // Keep the cursor on the same server as the list changes.
                    let current = app.current().map(|s| s.id.clone());
                    app.snapshot = Some(*snapshot);
                    let servers = app.servers();
                    let previous = app.table.selected().unwrap_or(0);
                    let index = current
                        .and_then(|id| servers.iter().position(|s| s.id == id))
                        .unwrap_or_else(|| previous.min(servers.len().saturating_sub(1)));
                    let live: HashSet<String> = servers.iter().map(|s| s.id.clone()).collect();
                    app.table.select(Some(index));
                    app.selected.retain(|id| live.contains(id));
                }
                Message::Done(text) => app.say(text),
            }
        }
        terminal
            .draw(|frame| draw(frame, app))
            .map_err(|e| e.to_string())?;
        if event::poll(Duration::from_millis(150)).map_err(|e| e.to_string())? {
            match event::read().map_err(|e| e.to_string())? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    app.key(key.code, key.modifiers)
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => app.click(mouse.column, mouse.row),
                    MouseEventKind::ScrollDown => app.move_cursor(1),
                    MouseEventKind::ScrollUp => app.move_cursor(-1),
                    _ => {}
                },
                _ => {}
            }
        }
    }
    Ok(())
}

fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}

fn memory_bar(snapshot: &Snapshot, width: u16) -> Line<'static> {
    let memory = &snapshot.memory;
    if memory.total == 0 || width == 0 {
        return Line::default();
    }
    let used = memory.total.saturating_sub(memory.available);
    let servers = memory.servers.min(used);
    let cells = width as f64;
    let mut server_cells = (servers as f64 / memory.total as f64 * cells).round() as u16;
    if servers > 0 {
        server_cells = server_cells.max(1);
    }
    let other_cells =
        ((used as f64 / memory.total as f64 * cells).round() as u16).saturating_sub(server_cells);
    let free_cells = width.saturating_sub(server_cells + other_cells);
    Line::from(vec![
        Span::styled(
            "━".repeat(server_cells as usize),
            Style::new().fg(Color::Cyan),
        ),
        Span::styled(
            "━".repeat(other_cells as usize),
            Style::new().fg(Color::Gray),
        ),
        Span::styled("━".repeat(free_cells as usize), dim()),
    ])
}

fn draw(frame: &mut Frame, app: &mut App) {
    let [header, bar, legend, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let Some(snapshot) = app.snapshot.as_ref() else {
        frame.render_widget(Paragraph::new("Scanning for dev servers…"), header);
        return;
    };
    let leaks = snapshot.servers.iter().filter(|s| s.leak.is_some()).count();
    let count = snapshot.servers.len();
    let mut title = vec![
        Span::styled(" oport ", Style::new().bold()),
        Span::raw(format!(
            " {count} dev server{} · {}",
            if count == 1 { "" } else { "s" },
            format_bytes(snapshot.memory.servers)
        )),
    ];
    if leaks > 0 {
        title.push(Span::styled(
            format!(
                "  ⚠ {leaks} memory warning{}",
                if leaks == 1 { "" } else { "s" }
            ),
            Style::new().fg(WARNING),
        ));
    }
    if snapshot.hidden > 0 {
        title.push(Span::styled(
            format!("  · {} other listener(s) hidden", snapshot.hidden),
            dim(),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(title)), header);
    frame.render_widget(
        Paragraph::new(memory_bar(snapshot, bar.width.saturating_sub(2))).centered(),
        bar,
    );
    let other = snapshot
        .memory
        .total
        .saturating_sub(snapshot.memory.available)
        .saturating_sub(snapshot.memory.servers);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ━ ", Style::new().fg(Color::Cyan)),
            Span::raw(format!("servers {}", format_bytes(snapshot.memory.servers))),
            Span::styled("   ━ ", Style::new().fg(Color::Gray)),
            Span::raw(format!("other apps {}", format_bytes(other))),
            Span::styled("   ━ ", dim()),
            Span::raw(format!("free {}", format_bytes(snapshot.memory.available))),
        ]))
        .style(dim()),
        legend,
    );

    match app.view {
        View::Details => match app.current().cloned() {
            Some(server) => draw_details(frame, body, &server, snapshot),
            None => app.view = View::List,
        },
        _ => draw_table(frame, body, app),
    }
    if app.view == View::Help {
        draw_help(frame, body);
    }

    let hint = match app.view {
        View::List => {
            "↑↓ select · enter details · o open · s stop · r restart · c clean up · ? keys · q quit"
        }
        View::Details => "esc back · o open · s stop · r restart · c clean up · ? keys",
        View::Cleanup => "space select · a all · n none · enter stop selected · esc cancel",
        View::Help => "any key closes",
    };
    let footer_line = if let Some((action, id)) = &app.confirm {
        let target = match action {
            Action::StopSelected => format!("{} selected server(s)", app.selected.len()),
            _ => app
                .servers()
                .iter()
                .find(|s| &s.id == id)
                .map(|s| {
                    format!(
                        "{}{}",
                        actions::describe(s),
                        if s.protected { " (protected)" } else { "" }
                    )
                })
                .unwrap_or_default(),
        };
        let verb = if *action == Action::Restart {
            "Restart"
        } else {
            "Stop"
        };
        Line::from(vec![
            Span::styled(
                format!(" {verb} {target}? "),
                Style::new().fg(WARNING).bold(),
            ),
            Span::raw("y / n"),
        ])
    } else if let Some((text, _)) = app
        .status
        .as_ref()
        .filter(|(_, at)| at.elapsed() < Duration::from_secs(6))
    {
        Line::from(Span::raw(format!(" {text}")))
    } else {
        Line::from(Span::styled(format!(" {hint}"), dim()))
    };
    frame.render_widget(Paragraph::new(footer_line), footer);
}

fn draw_table(frame: &mut Frame, area: Rect, app: &mut App) {
    let cleanup = app.view == View::Cleanup;
    let snapshot = app.snapshot.as_ref().expect("snapshot");
    let rows: Vec<Row> = snapshot
        .servers
        .iter()
        .map(|s| {
            let state = if let Some(leak) = &s.leak {
                Span::styled(leak.clone(), Style::new().fg(WARNING))
            } else if s.cwd_missing {
                Span::styled("folder deleted", Style::new().fg(Color::Red))
            } else if s.idle {
                Span::styled(
                    format!("idle {}", format_duration(s.idle_seconds.unwrap_or(0))),
                    dim(),
                )
            } else if s.protected {
                Span::styled("protected", dim())
            } else {
                Span::raw("")
            };
            let mut cells = vec![
                Cell::from(Span::styled(
                    match s.ports.len() {
                        1 => format!(":{}", s.ports[0]),
                        n => format!(":{} +{}", s.ports[0], n - 1),
                    },
                    Style::new().fg(port_color(s.ports[0])).bold(),
                )),
                Cell::from(s.name.clone()),
                Cell::from(Span::styled(s.branch.clone().unwrap_or_default(), dim())),
                Cell::from(s.framework.clone().unwrap_or_else(|| s.runtime.clone())),
                Cell::from(s.agent.as_ref().map(|a| a.label).unwrap_or("")),
                Cell::from(Line::from(format_duration(s.uptime)).right_aligned()),
                Cell::from(Line::from(format_bytes(s.memory)).right_aligned().style(
                    if s.leak.is_some() {
                        Style::new().fg(WARNING)
                    } else {
                        Style::new()
                    },
                )),
                Cell::from(Line::from(format!("{:.1}%", s.cpu)).right_aligned()),
                Cell::from(state),
            ];
            if cleanup {
                let mark = if s.protected {
                    "   "
                } else if app.selected.contains(&s.id) {
                    "[x]"
                } else {
                    "[ ]"
                };
                cells.insert(0, Cell::from(mark));
            }
            let row = Row::new(cells);
            if s.cwd_missing {
                row.add_modifier(Modifier::DIM)
            } else {
                row
            }
        })
        .collect();
    let mut widths = vec![
        Constraint::Length(10),
        Constraint::Fill(2),
        Constraint::Fill(1),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(7),
        Constraint::Length(10),
        Constraint::Length(6),
        Constraint::Fill(2),
    ];
    let mut header = vec![
        "PORT",
        "PROJECT",
        "BRANCH",
        "FRAMEWORK",
        "AGENT",
        "UP",
        "MEMORY",
        "CPU",
        "",
    ];
    if cleanup {
        widths.insert(0, Constraint::Length(3));
        header.insert(0, "");
    }
    let title = if cleanup {
        format!(
            " Clean up · {} selected · folders deleted or idle are preselected ",
            app.selected.len()
        )
    } else {
        " Servers ".to_string()
    };
    let empty = rows.is_empty();
    let table = Table::new(rows, widths)
        .header(Row::new(header).style(dim().bold()))
        .row_highlight_style(Style::new().reversed())
        .column_spacing(2)
        .block(
            Block::new()
                .borders(Borders::TOP)
                .title(title)
                .border_style(dim()),
        );
    app.table_area = area;
    frame.render_stateful_widget(table, area, &mut app.table);
    if empty {
        let [_, _, message] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);
        frame.render_widget(
            Paragraph::new(format!(
                " No dev servers listening on ports {}–{}.",
                snapshot.limits.port_min, snapshot.limits.port_max
            ))
            .style(dim()),
            message,
        );
    }
}

fn draw_details(frame: &mut Frame, area: Rect, server: &Server, snapshot: &Snapshot) {
    let color = port_color(server.ports[0]);
    let block = Block::new()
        .borders(Borders::TOP)
        .border_style(dim())
        .title(Line::from(vec![
            Span::styled(
                format!(
                    " {} ",
                    server
                        .ports
                        .iter()
                        .map(|p| format!(":{p}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
                Style::new().fg(color).bold(),
            ),
            Span::styled(format!("{} ", server.name), Style::new().bold()),
        ]));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let field = |label: &str, value: String| {
        Line::from(vec![
            Span::styled(format!("{label:<12}"), dim()),
            Span::raw(value),
        ])
    };
    let mut lines = Vec::new();
    if let Some(leak) = &server.leak {
        lines.push(Line::from(Span::styled(
            format!("⚠ {leak}"),
            Style::new().fg(WARNING).bold(),
        )));
    }
    lines.push(field("URL", server.url.clone()));
    lines.push(field(
        "Framework",
        server
            .framework
            .clone()
            .unwrap_or_else(|| server.runtime.clone()),
    ));
    if let Some(agent) = &server.agent {
        let host = agent.host.map(|h| format!(" in {h}")).unwrap_or_default();
        lines.push(field("Agent", format!("{}{host}", agent.label)));
        if let Some(resume) = &agent.resume {
            lines.push(field("Resume", resume.clone()));
        }
    }
    if let Some(branch) = &server.branch {
        lines.push(field(
            "Branch",
            format!(
                "{branch}{}",
                if server.worktree { " (worktree)" } else { "" }
            ),
        ));
    }
    lines.push(field(
        "Folder",
        format!(
            "{}{}",
            home_relative(&server.cwd),
            if server.cwd_missing { " (deleted)" } else { "" }
        ),
    ));
    lines.push(field("Command", server.command.clone()));
    lines.push(field("Listening", server.addresses.join("  ")));
    let idle = server
        .idle_seconds
        .filter(|s| *s > 60)
        .map(|s| format!(" · idle {}", format_duration(s)))
        .unwrap_or_default();
    lines.push(field(
        "Up",
        format!(
            "{}{idle} · {} connection(s)",
            format_duration(server.uptime),
            server.connections
        ),
    ));
    if server.protected {
        lines.push(field("Protected", "stopping asks for confirmation".into()));
    }

    let text_height = lines.len() as u16 + 1;
    let [text, memory_label, memory, cpu_label, cpu, processes] = Layout::vertical([
        Constraint::Length(text_height),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(2),
    ])
    .areas(inner);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), text);

    let growth = server
        .growth
        .filter(|g| *g > 0)
        .map(|g| {
            format!(
                " · +{} in {} min",
                format_bytes(g as u64),
                snapshot.limits.window_minutes
            )
        })
        .unwrap_or_default();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Memory      ", dim()),
            Span::raw(format!("{}{growth}", format_bytes(server.memory))),
            Span::styled("   last 10 minutes", dim()),
        ])),
        memory_label,
    );
    let memory_tint = if server.leak.is_some() {
        WARNING
    } else {
        Color::Cyan
    };
    frame.render_widget(
        Sparkline::default()
            .data(&server.history.memory)
            .max(
                server
                    .history
                    .memory
                    .iter()
                    .copied()
                    .max()
                    .unwrap_or(1)
                    .max(snapshot.limits.memory_limit / 4),
            )
            .style(Style::new().fg(memory_tint)),
        memory,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("CPU         ", dim()),
            Span::raw(format!("{:.1}%", server.cpu)),
        ])),
        cpu_label,
    );
    let cpu_data: Vec<u64> = server
        .history
        .cpu
        .iter()
        .map(|c| (c * 10.0) as u64)
        .collect();
    frame.render_widget(
        Sparkline::default()
            .data(&cpu_data)
            .max(cpu_data.iter().copied().max().unwrap_or(0).max(250))
            .style(Style::new().fg(Color::Blue)),
        cpu,
    );

    let rows: Vec<Row> = server
        .processes
        .iter()
        .map(|p| {
            Row::new(vec![
                Cell::from(if p.listening { "●" } else { " " }),
                Cell::from(p.pid.to_string()),
                Cell::from(Line::from(format_bytes(p.memory)).right_aligned()),
                Cell::from(Line::from(format!("{:.1}%", p.cpu)).right_aligned()),
                Cell::from(p.command.clone()),
            ])
        })
        .collect();
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(1),
                Constraint::Length(8),
                Constraint::Length(10),
                Constraint::Length(6),
                Constraint::Fill(1),
            ],
        )
        .header(Row::new(vec!["", "PID", "MEMORY", "CPU", "COMMAND"]).style(dim().bold()))
        .column_spacing(2)
        .block(
            Block::new()
                .borders(Borders::TOP)
                .title(" Processes · ● listening ")
                .border_style(dim()),
        ),
        processes,
    );
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let keys = [
        ("↑ ↓  j k", "Select a server"),
        ("enter  l", "Details"),
        ("esc  h", "Back"),
        ("o", "Open in the browser"),
        ("s", "Stop the server's process tree"),
        ("r", "Restart: stop, then rerun its command"),
        ("c", "Clean up: stop several servers at once"),
        ("space", "Select in clean up"),
        ("mouse", "Click to select, click again to open"),
        ("q", "Quit"),
    ];
    let lines: Vec<Line> = keys
        .iter()
        .map(|(key, what)| {
            Line::from(vec![
                Span::styled(format!("  {key:<12}"), Style::new().bold()),
                Span::raw(*what),
            ])
        })
        .collect();
    let width = 56.min(area.width);
    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Keys ")),
        popup,
    );
}
