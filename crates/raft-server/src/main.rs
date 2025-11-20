use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::io;
use std::thread;
use std::time::Duration;

use clap::Parser;
use crossbeam_channel::{unbounded, Receiver, Sender};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Terminal;
use slog::{o, Drain, Logger};

use crate::cli::{parse_peers, Args};
use crate::server::RaftKvService;
use raft_core::commands::{parse_command, ServerCommand};
use raft_core::node::Node;
use raft_core::raft_loop::{RaftDriver, StateUpdate};
use raft_core::raft_node::{RaftNode, RaftState};
use raft_core::storage::RaftStorage;
use raft_core::tcp_transport::TcpTransport;
use raft_proto::rpc::key_value_service_server::KeyValueServiceServer;
use tonic::transport::Server;

pub mod cli;
pub mod server;

/// Input mode for the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputMode {
    /// Normal mode - press 'i' to edit, 'q' to quit
    Normal,
    /// Editing mode - type commands, Enter to submit, Esc to return to Normal
    Editing,
}

/// Application state for the TUI.
struct App {
    /// Node ID
    node_id: u64,

    /// Current input buffer
    input: String,
    /// Cursor position in input buffer
    cursor_position: usize,
    /// Current input mode
    input_mode: InputMode,

    /// Latest Raft state (None until first update)
    raft_state: Option<RaftState>,
    /// Current KV store snapshot (built from StateUpdate::KvUpdate messages)
    kv_store: BTreeMap<String, String>,

    /// System messages (ring buffer, max 100)
    system_messages: VecDeque<String>,
    /// Command history (ring buffer, max 50)
    command_history: VecDeque<String>,

    /// Channel to receive state updates from Raft thread
    state_rx: Receiver<StateUpdate>,
    /// Channel to send commands to Raft thread
    cmd_tx: Sender<ServerCommand>,
    /// Channel to trigger shutdown
    shutdown_tx: Sender<()>,
}

impl App {
    /// Create a new App instance.
    fn new(
        node_id: u64,
        state_rx: Receiver<StateUpdate>,
        cmd_tx: Sender<ServerCommand>,
        shutdown_tx: Sender<()>,
    ) -> Self {
        Self {
            node_id,
            input: String::new(),
            cursor_position: 0,
            input_mode: InputMode::Editing, // Start in editing mode for convenience
            raft_state: None,
            kv_store: BTreeMap::new(),
            system_messages: VecDeque::with_capacity(100),
            command_history: VecDeque::with_capacity(50),
            state_rx,
            cmd_tx,
            shutdown_tx,
        }
    }

    /// Apply a state update from the Raft thread.
    fn apply_state_update(&mut self, update: StateUpdate) {
        match update {
            StateUpdate::RaftState(state) => {
                self.raft_state = Some(state);
            }
            StateUpdate::KvUpdate { key, value } => {
                self.kv_store.insert(key.clone(), value.clone());
                self.add_system_message(format!("KV Update: {} = {}", key, value));
            }
            StateUpdate::LogEntry { index, term, data } => {
                self.add_system_message(format!(
                    "Log Entry: idx={} term={} data={}",
                    index, term, data
                ));
            }
            StateUpdate::SystemMessage(msg) => {
                self.add_system_message(msg);
            }
        }
    }

    /// Add a system message to the ring buffer.
    fn add_system_message(&mut self, msg: String) {
        if self.system_messages.len() >= 100 {
            self.system_messages.pop_front();
        }
        self.system_messages.push_back(msg);
    }

    /// Add a command to the history ring buffer.
    fn add_command_history(&mut self, cmd: String) {
        if self.command_history.len() >= 50 {
            self.command_history.pop_front();
        }
        self.command_history.push_back(cmd);
    }

    /// Handle a key event and return the action to take.
    fn handle_key_event(&mut self, key: KeyEvent) -> Result<KeyAction, Box<dyn Error>> {
        match self.input_mode {
            InputMode::Normal => match key.code {
                KeyCode::Char('q') => Ok(KeyAction::Quit),
                KeyCode::Char('i') => {
                    self.input_mode = InputMode::Editing;
                    Ok(KeyAction::Continue)
                }
                _ => Ok(KeyAction::Continue),
            },
            InputMode::Editing => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    Ok(KeyAction::Continue)
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Ok(KeyAction::Quit)
                }
                KeyCode::Enter => self.submit_command(),
                KeyCode::Backspace => {
                    if self.cursor_position > 0 {
                        self.input.remove(self.cursor_position - 1);
                        self.cursor_position -= 1;
                    }
                    Ok(KeyAction::Continue)
                }
                KeyCode::Left => {
                    if self.cursor_position > 0 {
                        self.cursor_position -= 1;
                    }
                    Ok(KeyAction::Continue)
                }
                KeyCode::Right => {
                    if self.cursor_position < self.input.len() {
                        self.cursor_position += 1;
                    }
                    Ok(KeyAction::Continue)
                }
                KeyCode::Char(c) => {
                    self.input.insert(self.cursor_position, c);
                    self.cursor_position += 1;
                    Ok(KeyAction::Continue)
                }
                _ => Ok(KeyAction::Continue),
            },
        }
    }

    /// Submit the current input as a command.
    fn submit_command(&mut self) -> Result<KeyAction, Box<dyn Error>> {
        let input = self.input.drain(..).collect::<String>();
        self.cursor_position = 0;

        if input.trim().is_empty() {
            return Ok(KeyAction::Continue);
        }

        // Handle quit/exit
        if input.trim() == "quit" || input.trim() == "exit" {
            return Ok(KeyAction::Quit);
        }

        // Parse and send command
        match parse_command(&input) {
            Some(cmd) => {
                self.cmd_tx.send(ServerCommand::User(cmd))?;
                self.add_command_history(input.clone());
                self.add_system_message(format!("Command sent: {}", input));
                Ok(KeyAction::Continue)
            }
            None => {
                self.add_system_message(format!("Unknown command: {}", input));
                Ok(KeyAction::Continue)
            }
        }
    }
}

/// Result of handling a key event.
enum KeyAction {
    Continue,
    Quit,
}

fn main() -> Result<(), Box<dyn Error>> {
    // 1. Parse CLI arguments
    let args = Args::parse();

    // 2. Setup logging (goes to file, not terminal)
    let logger = setup_logger(args.id);

    slog::info!(logger, "Starting Raft node";
                "node_id" => args.id,
                "peers" => &args.peers);

    // 3. Parse peer addresses
    let peers_map = parse_peers(&args.peers)?;

    // Verify our ID is in the peer list
    let my_addr = *peers_map
        .get(&args.id)
        .ok_or_else(|| format!("Node ID {} not found in peer list", args.id))?;

    // Get list of peer IDs for Raft initialization
    let peer_ids: Vec<u64> = peers_map.keys().copied().collect();

    slog::info!(logger, "Parsed peers";
                "my_addr" => format!("{}", my_addr),
                "peer_count" => peer_ids.len());

    eprintln!("Node {} starting on {}", args.id, my_addr);
    eprintln!("Cluster: {} nodes", peer_ids.len());

    // 4. Create Raft storage (persistent by default, or in-memory with :memory:)
    let storage = match args.data_dir.as_deref() {
        Some(":memory:") => {
            eprintln!("Using in-memory storage (no persistence)");
            slog::info!(logger, "Using in-memory storage");
            RaftStorage::new()
        }
        data_dir => {
            let path = data_dir
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("./data/node-{}", args.id));

            // Create directory if it doesn't exist
            std::fs::create_dir_all(&path)?;

            eprintln!("Using persistent storage: {}", path);
            slog::info!(logger, "Using persistent storage"; "path" => &path);

            RaftStorage::new_with_disk(&path)?
        }
    };

    // 5. Create Raft node
    let raft_node = RaftNode::new(args.id, peer_ids, storage, logger.clone())?;
    let kv_node = Node::new();

    // 6. Create channels
    let (cmd_tx, cmd_rx) = unbounded();
    let (msg_tx, msg_rx) = unbounded();
    let (state_tx, state_rx) = unbounded();
    let (shutdown_tx, shutdown_rx) = unbounded();

    // 7. Create TCP transport
    let transport = TcpTransport::new(args.id, my_addr, peers_map.clone(), msg_tx, logger.clone())?;

    slog::info!(logger, "Created TCP transport"; "listen_addr" => format!("{}", my_addr));

    eprintln!("Transport listening on {}", my_addr);
    eprintln!();

    // Calculate gRPC address (port + 1000)
    let mut grpc_addr = my_addr;
    grpc_addr.set_port(grpc_addr.port() + 1000);

    let rpc_cmd_tx = cmd_tx.clone();
    // Create a map of peers with their gRPC ports (raft port + 1000)
    // This ensures clients are redirected to the API port, not the internal Raft port
    let rpc_peers_map: std::collections::HashMap<_, _> = peers_map
        .iter()
        .map(|(id, addr)| {
            let mut addr = *addr;
            addr.set_port(addr.port() + 1000);
            (*id, addr)
        })
        .collect();

    // Spawn gRPC server in background thread
    thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let service = RaftKvService {
                cmd_tx: rpc_cmd_tx,
                peers_map: rpc_peers_map,
            };

            eprintln!("gRPC API listening on {}", grpc_addr);

            Server::builder()
                .add_service(KeyValueServiceServer::new(service))
                .serve(grpc_addr)
                .await
                .unwrap();
        });
    });

    // 8. Spawn Raft ready loop thread
    let raft_logger = logger.clone();
    let raft_handle = thread::Builder::new()
        .name(format!("raft-ready-loop-{}", args.id))
        .spawn(move || {
            slog::info!(raft_logger, "Raft ready loop starting");
            let driver = RaftDriver::new(
                raft_node,
                kv_node,
                cmd_rx,
                msg_rx,
                state_tx,
                transport,
                shutdown_rx,
            );
            let result = driver.run();

            match &result {
                Ok(_) => slog::info!(raft_logger, "Raft ready loop exited cleanly"),
                Err(e) => {
                    slog::error!(raft_logger, "Raft ready loop error"; "error" => format!("{}", e))
                }
            }

            result
        })?;

    // 9. Create and run TUI
    let mut app = App::new(args.id, state_rx, cmd_tx, shutdown_tx);
    let loop_result = run_tui(&mut app);

    // 10. Wait for Raft thread to exit
    slog::info!(logger, "Waiting for Raft thread to exit");
    let raft_result = raft_handle.join().expect("Raft thread panicked");

    // Return first error if any
    loop_result?;
    raft_result?;

    slog::info!(logger, "Node shutdown complete");
    Ok(())
}

/// Setup slog logger with file output.
///
/// Logs are written to `node-{id}.log` in the current directory.
/// This keeps the interactive terminal clean while still capturing
/// all Raft events for debugging.
fn setup_logger(node_id: u64) -> Logger {
    use std::fs::OpenOptions;

    let log_path = format!("node-{}.log", node_id);

    // Open log file (create or truncate)
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
        .unwrap_or_else(|e| {
            eprintln!("Failed to open log file {}: {}", log_path, e);
            std::process::exit(1);
        });

    let decorator = slog_term::PlainDecorator::new(file);
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();

    eprintln!("Logging to: {}", log_path);

    Logger::root(drain, o!("node_id" => node_id))
}

/// Render the TUI.
fn ui(frame: &mut ratatui::Frame, app: &App) {
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title bar
            Constraint::Min(10),   // Main content
            Constraint::Length(3), // Input area
        ])
        .split(frame.area());

    // Title bar
    draw_title_bar(frame, app, main_chunks[0]);

    // Split main area horizontally
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50), // Left: Raft state + logs
            Constraint::Percentage(50), // Right: KV store + history
        ])
        .split(main_chunks[1]);

    // Split left pane vertically
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40), // Raft state
            Constraint::Percentage(60), // System logs
        ])
        .split(content_chunks[0]);

    draw_raft_state(frame, app, left_chunks[0]);
    draw_system_logs(frame, app, left_chunks[1]);

    // Split right pane vertically
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50), // KV store
            Constraint::Percentage(50), // Command history
        ])
        .split(content_chunks[1]);

    draw_kv_store(frame, app, right_chunks[0]);
    draw_command_history(frame, app, right_chunks[1]);

    // Input area at bottom
    draw_input_area(frame, app, main_chunks[2]);
}

/// Draw the title bar.
fn draw_title_bar(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let title = if let Some(ref state) = app.raft_state {
        format!(
            "Node {} | Term: {} | Role: {:?} | Leader: {}",
            app.node_id, state.term, state.role, state.leader_id
        )
    } else {
        format!("Node {} | Initializing...", app.node_id)
    };

    let title_widget = Paragraph::new(title)
        .block(Block::default().borders(Borders::ALL).title("Raft-a-TUI"))
        .style(Style::default().fg(Color::Cyan));

    frame.render_widget(title_widget, area);
}

/// Draw the Raft state panel.
fn draw_raft_state(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let text = if let Some(ref state) = app.raft_state {
        format!(
            "Term: {}\nRole: {:?}\nLeader ID: {}\nCluster Size: {}\nCommit Index: {}\nApplied Index: {}",
            state.term, state.role, state.leader_id, state.cluster_size, state.commit_index, state.applied_index
        )
    } else {
        "Waiting for Raft state...".to_string()
    };

    let widget = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title("Raft State"))
        .wrap(Wrap { trim: true });

    frame.render_widget(widget, area);
}

/// Draw the system logs panel.
fn draw_system_logs(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .system_messages
        .iter()
        .rev() // Most recent first
        .take(area.height as usize - 2) // Fit in visible area
        .map(|msg| ListItem::new(msg.as_str()))
        .collect();

    let widget = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("System Logs"))
        .style(Style::default().fg(Color::Gray));

    frame.render_widget(widget, area);
}

/// Draw the KV store panel.
fn draw_kv_store(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .kv_store
        .iter()
        .map(|(k, v)| ListItem::new(format!("{} = {}", k, v)))
        .collect();

    let widget = if items.is_empty() {
        List::new(vec![ListItem::new("(empty)")])
            .block(Block::default().borders(Borders::ALL).title("KV Store"))
            .style(Style::default().fg(Color::DarkGray))
    } else {
        List::new(items)
            .block(Block::default().borders(Borders::ALL).title("KV Store"))
            .style(Style::default().fg(Color::Green))
    };

    frame.render_widget(widget, area);
}

/// Draw the command history panel.
fn draw_command_history(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .command_history
        .iter()
        .rev() // Most recent first
        .take(area.height as usize - 2)
        .map(|cmd| ListItem::new(cmd.as_str()))
        .collect();

    let widget = if items.is_empty() {
        List::new(vec![ListItem::new("(no commands yet)")])
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Command History"),
            )
            .style(Style::default().fg(Color::DarkGray))
    } else {
        List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Command History"),
            )
            .style(Style::default().fg(Color::Yellow))
    };

    frame.render_widget(widget, area);
}

/// Draw the input area.
fn draw_input_area(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let (title, style) = match app.input_mode {
        InputMode::Normal => ("Press 'i' to enter command, 'q' to quit", Style::default()),
        InputMode::Editing => (
            "Type command, Enter to submit, Esc to cancel",
            Style::default().fg(Color::Yellow),
        ),
    };

    let input_widget = Paragraph::new(app.input.as_str())
        .block(Block::default().borders(Borders::ALL).title(title))
        .style(style);

    frame.render_widget(input_widget, area);

    // Set cursor position in Editing mode
    if app.input_mode == InputMode::Editing {
        frame.set_cursor_position((
            area.x + app.cursor_position as u16 + 1, // +1 for border
            area.y + 1,                              // +1 for border
        ));
    }
}

/// Setup terminal for TUI rendering.
fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>, Box<dyn Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

/// Restore terminal to normal mode.
fn restore_terminal(
    mut terminal: Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), Box<dyn Error>> {
    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Run the TUI main loop.
fn run_tui(app: &mut App) -> Result<(), Box<dyn Error>> {
    let mut terminal = setup_terminal()?;

    let result = (|| -> Result<(), Box<dyn Error>> {
        loop {
            // 1. Drain all pending state updates (non-blocking)
            while let Ok(update) = app.state_rx.try_recv() {
                app.apply_state_update(update);
            }

            // 2. Draw current state
            terminal.draw(|frame| ui(frame, app))?;

            // 3. Poll for keyboard input with timeout
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    match app.handle_key_event(key)? {
                        KeyAction::Quit => {
                            app.shutdown_tx.send(())?;
                            break;
                        }
                        KeyAction::Continue => {}
                    }
                }
            }
        }

        Ok(())
    })();

    // Always restore terminal
    restore_terminal(terminal)?;

    result
}
