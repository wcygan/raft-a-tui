use std::error::Error;
use std::io;
use std::time::Duration;

use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Terminal;
use tokio::time::sleep;
use tonic::Request;

use raft_proto::rpc::key_value_service_client::KeyValueServiceClient;
use raft_proto::rpc::{GetRequest, PutRequest};

#[derive(Parser, Debug)]
#[command(name = "raft-client")]
struct Args {
    /// List of gRPC peer addresses (e.g., http://127.0.0.1:7001,http://127.0.0.1:7002)
    #[arg(long)]
    peers: String,
}

enum InputMode {
    Normal,
    Editing,
}

struct App {
    input: String,
    input_mode: InputMode,
    messages: Vec<String>,

    // Client state
    peers: Vec<String>,
    leader_addr: Option<String>, // Hint for current leader
}

impl App {
    fn new(peers: Vec<String>) -> Self {
        Self {
            input: String::new(),
            input_mode: InputMode::Editing,
            messages: Vec::new(),
            peers,
            leader_addr: None,
        }
    }

    /// Get the address to connect to (leader hint or random peer).
    fn get_target_addr(&self) -> String {
        if let Some(ref leader) = self.leader_addr {
            return leader.clone();
        }
        // Pick random (or round robin, here just first for simplicity or random)
        use rand::seq::SliceRandom;
        let mut rng = rand::thread_rng();
        self.peers
            .choose(&mut rng)
            .unwrap_or(&self.peers[0])
            .clone()
    }

    async fn handle_command(&mut self, cmd: String) {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.is_empty() {
            return;
        }

        let start = std::time::Instant::now();
        match parts[0].to_ascii_lowercase().as_str() {
            "put" | "p" => {
                if parts.len() < 3 {
                    self.messages.push("Usage: put <key> <value>".to_string());
                    return;
                }
                let key = parts[1].to_string();
                let value = parts[2..].join(" ");
                match self.put(key, value).await {
                    Ok(_) => self
                        .messages
                        .push(format!("PUT Success ({:?})", start.elapsed())),
                    Err(e) => self.messages.push(format!("PUT Error: {}", e)),
                }
            }
            "get" | "g" => {
                if parts.len() != 2 {
                    self.messages.push("Usage: get <key>".to_string());
                    return;
                }
                let key = parts[1].to_string();
                match self.get(key).await {
                    Ok(val) => self
                        .messages
                        .push(format!("GET: {} ({:?})", val, start.elapsed())),
                    Err(e) => self.messages.push(format!("GET Error: {}", e)),
                }
            }
            "quit" | "exit" | "q" => {
                // Handled in main loop
            }
            _ => {
                self.messages.push(format!("Unknown command: {}", parts[0]));
            }
        }
    }

    async fn put(&mut self, key: String, value: String) -> Result<(), String> {
        for _attempt in 0..5 {
            let addr = self.get_target_addr();
            // Ensure http:// prefix
            let addr = if addr.starts_with("http") {
                addr.clone()
            } else {
                format!("http://{}", addr)
            };

            let mut client = match KeyValueServiceClient::connect(addr.clone()).await {
                Ok(c) => c,
                Err(_e) => {
                    // Network error, try another peer next time
                    self.leader_addr = None; // Clear hint
                    continue;
                }
            };

            let request = Request::new(PutRequest {
                key: key.clone(),
                value: value.clone(),
            });

            match client.put(request).await {
                Ok(_) => return Ok(()),
                Err(status) => {
                    if status.code() == tonic::Code::FailedPrecondition {
                        // Check for redirection metadata
                        if let Some(new_addr_val) = status.metadata().get("x-raft-leader-addr") {
                            if let Ok(new_addr) = new_addr_val.to_str() {
                                let new_addr = new_addr.to_string();
                                // The server returns raw IP:PORT (e.g. 127.0.0.1:6001).
                                // We need to map this to the gRPC port!
                                // Wait, the server returns the Raft peer address.
                                // The client needs to know the mapping from Raft Addr -> gRPC Addr.
                                // Assumption: gRPC port = Raft port + 1000.
                                // This is a hack but consistent with our server implementation.
                                if let Ok(socket_addr) = new_addr.parse::<std::net::SocketAddr>() {
                                    let mut grpc_socket = socket_addr;
                                    grpc_socket.set_port(grpc_socket.port() + 1000);
                                    let grpc_url = format!("http://{}", grpc_socket);

                                    self.messages
                                        .push(format!("Redirecting to leader: {}", grpc_url));
                                    self.leader_addr = Some(grpc_url);
                                    continue; // Retry
                                }
                            }
                        }
                    }

                    // Other error, maybe leader election in progress, just retry with random
                    self.leader_addr = None;
                    sleep(Duration::from_millis(100)).await;
                }
            }
        }
        Err("Max retries exceeded".to_string())
    }

    async fn get(&mut self, key: String) -> Result<String, String> {
        // Reads can go anywhere, but let's just try target addr
        // Or we could randomly pick one every time for reads to distribute load
        let addr = self.get_target_addr(); // Or random
        let addr = if addr.starts_with("http") {
            addr.clone()
        } else {
            format!("http://{}", addr)
        };

        let mut client = KeyValueServiceClient::connect(addr)
            .await
            .map_err(|e| e.to_string())?;

        let request = Request::new(GetRequest { key });
        let response = client
            .get(request)
            .await
            .map_err(|e| e.message().to_string())?;
        let inner = response.into_inner();

        if inner.found {
            Ok(inner.value)
        } else {
            Ok("(not found)".to_string())
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    let peers: Vec<String> = args
        .peers
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    // Setup Terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(peers);

    loop {
        terminal.draw(|f| ui(f, &app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    break;
                }

                match app.input_mode {
                    InputMode::Normal => match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Char('i') => app.input_mode = InputMode::Editing,
                        _ => {}
                    },
                    InputMode::Editing => match key.code {
                        KeyCode::Esc => app.input_mode = InputMode::Normal,
                        KeyCode::Enter => {
                            let cmd = app.input.clone();
                            app.input.clear();
                            if cmd == "quit" || cmd == "exit" || cmd == "q" {
                                break;
                            }
                            app.handle_command(cmd).await;
                        }
                        KeyCode::Char(c) => {
                            app.input.push(c);
                        }
                        KeyCode::Backspace => {
                            app.input.pop();
                        }
                        _ => {}
                    },
                }
            }
        }
    }

    // Restore
    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}

fn ui(frame: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(frame.area());

    let messages: Vec<ListItem> = app
        .messages
        .iter()
        .rev()
        .map(|m| ListItem::new(m.as_str()))
        .collect();
    let list = List::new(messages).block(Block::default().borders(Borders::ALL).title("Log"));
    frame.render_widget(list, chunks[0]);

    let input = Paragraph::new(app.input.as_str())
        .style(match app.input_mode {
            InputMode::Normal => Style::default(),
            InputMode::Editing => Style::default().fg(Color::Yellow),
        })
        .block(Block::default().borders(Borders::ALL).title("Input"));
    frame.render_widget(input, chunks[1]);

    if matches!(app.input_mode, InputMode::Editing) {
        frame.set_cursor_position((chunks[1].x + app.input.len() as u16 + 1, chunks[1].y + 1));
    }
}
