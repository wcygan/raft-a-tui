# Ratatui Reference Documentation

Complete reference guide for building terminal user interfaces with ratatui.

## Documentation Index

### Official Ratatui Documentation

**Core Concepts:**
- Main site: https://ratatui.rs/
- Widgets: https://ratatui.rs/concepts/widgets/
- Layout: https://ratatui.rs/concepts/layout/
- Event Handling: https://ratatui.rs/concepts/event-handling/
- Rendering: https://ratatui.rs/concepts/rendering/
- Builder-Lite Pattern: https://ratatui.rs/concepts/builder-lite-pattern/
- Component Architecture: https://ratatui.rs/concepts/application-patterns/component-architecture/

**Examples:**
- User Input: https://ratatui.rs/examples/apps/user_input/
- Colors: https://ratatui.rs/examples/style/colors/
- RGB Colors: https://ratatui.rs/examples/style/colors_rgb/

**Layout Recipes:**
- Layout Overview: https://ratatui.rs/recipes/layout/
- Grid Layout: https://ratatui.rs/recipes/layout/grid/
- Dynamic Layouts: https://ratatui.rs/recipes/layout/dynamic/

**Application Recipes:**
- Apps Overview: https://ratatui.rs/recipes/apps/
- CLI Arguments: https://ratatui.rs/recipes/apps/cli-arguments/
- Logging with Tracing: https://ratatui.rs/recipes/apps/log-with-tracing/
- Terminal and Event Handler: https://ratatui.rs/recipes/apps/terminal-and-event-handler/

**Rendering Recipes:**
- Display Text: https://ratatui.rs/recipes/render/display-text/
- Style Text: https://ratatui.rs/recipes/render/style-text/

---

## Detailed Widget Reference

### Block Widget

The foundation widget providing borders, titles, and padding:

```rust
use ratatui::widgets::{Block, Borders, BorderType};
use ratatui::style::{Style, Color};

let block = Block::default()
    .borders(Borders::ALL)
    .border_type(BorderType::Rounded)
    .border_style(Style::default().fg(Color::Cyan))
    .title("My Title")
    .title_alignment(Alignment::Center)
    .title_style(Style::default().fg(Color::Yellow).bold());

// Available border types:
// - BorderType::Plain
// - BorderType::Rounded
// - BorderType::Double
// - BorderType::Thick
```

**Use Cases:**
- Container for other widgets
- Visual separation of UI sections
- Titled panels

### Paragraph Widget

Renders text with wrapping and alignment:

```rust
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::text::Text;

let text = Text::from(vec![
    Line::from("First line"),
    Line::from(vec![
        Span::raw("Second line with "),
        Span::styled("color", Style::default().fg(Color::Green)),
    ]),
]);

let paragraph = Paragraph::new(text)
    .block(Block::bordered().title("Info"))
    .style(Style::default().fg(Color::White))
    .alignment(Alignment::Left)
    .wrap(Wrap { trim: true });

frame.render_widget(paragraph, area);
```

**Key Features:**
- Automatic text wrapping with `Wrap`
- Alignment: Left, Center, Right
- Scroll support (when combined with state)

### List Widget (Stateful)

Displays selectable items vertically:

```rust
use ratatui::widgets::{List, ListItem, ListState};

let items: Vec<ListItem> = vec![
    ListItem::new("Item 1"),
    ListItem::new("Item 2").style(Style::default().fg(Color::Yellow)),
    ListItem::new(Line::from(vec![
        Span::raw("Item 3 with "),
        Span::styled("color", Style::default().fg(Color::Green)),
    ])),
];

let list = List::new(items)
    .block(Block::bordered().title("Items"))
    .highlight_style(Style::default().bg(Color::DarkGray))
    .highlight_symbol(">> ");

// Maintain list state for selection
let mut list_state = ListState::default();
list_state.select(Some(0)); // Select first item

frame.render_stateful_widget(list, area, &mut list_state);

// Navigation in event handler
match key.code {
    KeyCode::Down => {
        let i = list_state.selected().map_or(0, |i| (i + 1) % items.len());
        list_state.select(Some(i));
    }
    KeyCode::Up => {
        let i = list_state.selected().map_or(0, |i| {
            if i == 0 { items.len() - 1 } else { i - 1 }
        });
        list_state.select(Some(i));
    }
    _ => {}
}
```

**Key Features:**
- Selection highlighting
- Custom highlight symbols
- Scrolling support

### Table Widget (Stateful)

Displays data in rows and columns:

```rust
use ratatui::widgets::{Table, Row, Cell, TableState};

let header = Row::new(vec!["Header 1", "Header 2", "Header 3"])
    .style(Style::default().fg(Color::Yellow).bold())
    .height(1);

let rows = vec![
    Row::new(vec!["Cell 1", "Cell 2", "Cell 3"]),
    Row::new(vec!["Cell 4", "Cell 5", "Cell 6"])
        .style(Style::default().fg(Color::Green)),
];

let table = Table::new(rows, [
    Constraint::Percentage(33),
    Constraint::Percentage(33),
    Constraint::Percentage(34),
])
    .header(header)
    .block(Block::bordered().title("Table"))
    .highlight_style(Style::default().bg(Color::DarkGray))
    .highlight_symbol(">> ");

let mut table_state = TableState::default();
table_state.select(Some(0));

frame.render_stateful_widget(table, area, &mut table_state);
```

**Key Features:**
- Column widths via constraints
- Row selection
- Header styling
- Cell-level styling

### Gauge Widget

Visualizes progress or metrics:

```rust
use ratatui::widgets::Gauge;

let gauge = Gauge::default()
    .block(Block::bordered().title("Progress"))
    .gauge_style(Style::default().fg(Color::Green).bg(Color::Black))
    .label(format!("{:.1}%", progress * 100.0))
    .ratio(progress); // 0.0 to 1.0

frame.render_widget(gauge, area);
```

**Use Cases:**
- Progress bars
- Capacity indicators
- Metric visualization

### BarChart Widget

Displays data as vertical bars:

```rust
use ratatui::widgets::BarChart;

let data = vec![
    ("Jan", 10),
    ("Feb", 20),
    ("Mar", 15),
];

let barchart = BarChart::default()
    .block(Block::bordered().title("Monthly Data"))
    .data(&data)
    .bar_width(3)
    .bar_gap(1)
    .bar_style(Style::default().fg(Color::Yellow))
    .value_style(Style::default().fg(Color::White).bg(Color::Black));

frame.render_widget(barchart, area);
```

**Key Features:**
- Grouped or ungrouped bars
- Configurable bar width and spacing
- Value labels

### Tabs Widget

Creates tab navigation:

```rust
use ratatui::widgets::Tabs;

let titles = vec!["Tab1", "Tab2", "Tab3"];
let tabs = Tabs::new(titles)
    .block(Block::bordered().title("Tabs"))
    .select(selected_tab)
    .style(Style::default().fg(Color::White))
    .highlight_style(Style::default().fg(Color::Yellow).bold());

frame.render_widget(tabs, area);
```

### Canvas Widget

Custom drawing with shapes:

```rust
use ratatui::widgets::canvas::{Canvas, Line, Rectangle, Circle};

let canvas = Canvas::default()
    .block(Block::bordered().title("Canvas"))
    .x_bounds([0.0, 100.0])
    .y_bounds([0.0, 100.0])
    .paint(|ctx| {
        ctx.draw(&Rectangle {
            x: 10.0,
            y: 10.0,
            width: 30.0,
            height: 20.0,
            color: Color::Red,
        });
        ctx.draw(&Circle {
            x: 50.0,
            y: 50.0,
            radius: 15.0,
            color: Color::Blue,
        });
        ctx.draw(&Line {
            x1: 0.0,
            y1: 0.0,
            x2: 100.0,
            y2: 100.0,
            color: Color::Green,
        });
    });

frame.render_widget(canvas, area);
```

### Chart Widget

Line and scatter plots:

```rust
use ratatui::widgets::{Chart, Axis, Dataset, GraphType};

let datasets = vec![
    Dataset::default()
        .name("Data 1")
        .marker(symbols::Marker::Dot)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(Color::Cyan))
        .data(&[(0.0, 5.0), (1.0, 6.0), (2.0, 7.0)]),
];

let chart = Chart::new(datasets)
    .block(Block::bordered().title("Chart"))
    .x_axis(Axis::default()
        .title("X Axis")
        .bounds([0.0, 10.0])
        .labels(vec!["0".into(), "5".into(), "10".into()]))
    .y_axis(Axis::default()
        .title("Y Axis")
        .bounds([0.0, 10.0])
        .labels(vec!["0".into(), "5".into(), "10".into()]));

frame.render_widget(chart, area);
```

---

## Advanced Layout Techniques

### Centering a Widget

```rust
use ratatui::layout::Rect;

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

// Usage
let popup_area = centered_rect(60, 20, frame.area());
frame.render_widget(popup, popup_area);
```

### Dynamic Layouts

Create layouts based on runtime conditions:

```rust
fn create_dynamic_layout(&self, area: Rect) -> Vec<Rect> {
    let mut constraints = vec![Constraint::Length(3)]; // Header

    if self.show_sidebar {
        constraints.push(Constraint::Percentage(20)); // Sidebar
        constraints.push(Constraint::Percentage(80)); // Main
    } else {
        constraints.push(Constraint::Percentage(100)); // Full main
    }

    constraints.push(Constraint::Length(3)); // Footer

    Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area)
}
```

### Collapsing Borders

Prevent double borders when nesting widgets:

```rust
// Outer block with all borders
let outer = Block::default()
    .borders(Borders::ALL)
    .title("Outer");

// Inner blocks without overlapping borders
let inner_left = Block::default()
    .borders(Borders::TOP | Borders::BOTTOM | Borders::LEFT)
    .title("Left");

let inner_right = Block::default()
    .borders(Borders::TOP | Borders::BOTTOM | Borders::RIGHT)
    .title("Right");
```

---

## Advanced Event Handling

### Async Event Handler with Tokio

```rust
use tokio::sync::mpsc;
use crossterm::event::{EventStream, Event};
use futures::StreamExt;

pub struct EventHandler {
    sender: mpsc::UnboundedSender<Event>,
    receiver: mpsc::UnboundedReceiver<Event>,
}

impl EventHandler {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self { sender, receiver }
    }

    pub async fn start(&self) {
        let sender = self.sender.clone();
        tokio::spawn(async move {
            let mut reader = EventStream::new();
            while let Some(Ok(event)) = reader.next().await {
                if sender.send(event).is_err() {
                    break;
                }
            }
        });
    }

    pub async fn next(&mut self) -> Option<Event> {
        self.receiver.recv().await
    }
}

// Usage in main loop
let mut event_handler = EventHandler::new();
event_handler.start().await;

loop {
    terminal.draw(|f| ui(f, &app))?;

    if let Some(event) = event_handler.next().await {
        match event {
            Event::Key(key) => {
                if handle_key_event(key, &mut app)? {
                    break;
                }
            }
            Event::Resize(_, _) => {
                // Terminal resized - redraw handled automatically
            }
            _ => {}
        }
    }
}
```

### Mouse Event Handling

```rust
use crossterm::event::{MouseEvent, MouseEventKind};

fn handle_mouse_event(&mut self, mouse: MouseEvent, area: Rect) -> Result<()> {
    match mouse.kind {
        MouseEventKind::Down(button) => {
            if button == MouseButton::Left {
                let x = mouse.column;
                let y = mouse.row;

                // Check if click is within specific area
                if x >= area.x && x < area.x + area.width &&
                   y >= area.y && y < area.y + area.height {
                    self.handle_click_in_area(x - area.x, y - area.y)?;
                }
            }
        }
        MouseEventKind::ScrollDown => {
            self.scroll_offset += 1;
        }
        MouseEventKind::ScrollUp => {
            self.scroll_offset = self.scroll_offset.saturating_sub(1);
        }
        _ => {}
    }
    Ok(())
}

// Enable mouse capture in terminal setup
execute!(stdout, EnableMouseCapture)?;

// Disable on cleanup
execute!(stdout, DisableMouseCapture)?;
```

---

## Production Patterns

### Logging with Tracing

```rust
use tracing::{info, error, debug};
use tracing_subscriber;

// Initialize logging (write to file to avoid terminal conflicts)
fn init_logging() -> Result<()> {
    let log_file = std::fs::File::create("app.log")?;
    tracing_subscriber::fmt()
        .with_writer(log_file)
        .with_ansi(false)
        .init();
    Ok(())
}

// Use in code
info!("Application started");
debug!("Processing event: {:?}", event);
error!("Failed to connect: {}", err);
```

### Panic Hook for Clean Terminal Restore

```rust
use std::panic;
use std::io;

fn install_panic_hook() {
    let original_hook = panic::take_hook();

    panic::set_hook(Box::new(move |panic_info| {
        let mut stdout = io::stdout();
        let _ = crossterm::execute!(
            stdout,
            crossterm::terminal::LeaveAlternateScreen,
        );
        let _ = crossterm::terminal::disable_raw_mode();

        original_hook(panic_info);
    }));
}

// Call before running app
fn main() -> Result<()> {
    init_logging()?;
    install_panic_hook();

    let mut app = App::new();
    app.run()?;

    Ok(())
}
```

### CLI Arguments with Clap

```rust
use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Node ID
    #[arg(short, long)]
    id: u64,

    /// Peer addresses (format: id=host:port)
    #[arg(short, long, value_delimiter = ',')]
    peers: Vec<String>,

    /// Enable debug mode
    #[arg(short, long)]
    debug: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let mut app = App::new(args.id, args.peers)?;
    if args.debug {
        app.enable_debug_mode();
    }

    app.run()?;
    Ok(())
}
```

### Configuration Directories

```rust
use directories::ProjectDirs;
use std::fs;

fn get_config_dir() -> Result<PathBuf> {
    if let Some(proj_dirs) = ProjectDirs::from("com", "MyOrg", "MyApp") {
        let config_dir = proj_dirs.config_dir();
        fs::create_dir_all(config_dir)?;
        Ok(config_dir.to_path_buf())
    } else {
        Err("Could not determine config directory".into())
    }
}

fn load_config() -> Result<Config> {
    let config_path = get_config_dir()?.join("config.toml");

    if config_path.exists() {
        let contents = fs::read_to_string(config_path)?;
        Ok(toml::from_str(&contents)?)
    } else {
        Ok(Config::default())
    }
}
```

---

## Performance Optimization

### Minimize Rendering

Only redraw when state changes:

```rust
struct App {
    needs_redraw: bool,
    last_render: Instant,
    // ... other fields
}

impl App {
    fn run(&mut self) -> Result<()> {
        let mut terminal = setup_terminal()?;

        loop {
            // Only render if needed or timeout elapsed
            let should_render = self.needs_redraw ||
                self.last_render.elapsed() > Duration::from_millis(100);

            if should_render {
                terminal.draw(|f| self.draw(f))?;
                self.needs_redraw = false;
                self.last_render = Instant::now();
            }

            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    self.handle_key(key)?;
                    self.needs_redraw = true;
                }
            }
        }
    }
}
```

### Use WidgetRef for Reusable Widgets

```rust
use ratatui::widgets::WidgetRef;

// Implement WidgetRef instead of Widget for reuse
impl WidgetRef for MyWidget {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        // Render without consuming self
    }
}

// Can now render same widget multiple times
let widget = MyWidget::new();
frame.render_widget_ref(&widget, area1);
frame.render_widget_ref(&widget, area2);
```

### Efficient Text Updates

Use `Text::from` conversions efficiently:

```rust
// Inefficient - creates intermediate String
let text = Text::from(format!("Count: {}", count));

// Efficient - use Line with multiple Spans
let text = Text::from(Line::from(vec![
    Span::raw("Count: "),
    Span::raw(count.to_string()),
]));

// Or for simple cases
let text = Line::from(format!("Count: {}", count));
```

---

## Testing Strategies

### Unit Testing Rendering Logic

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn test_rendering() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        let app = App::new();

        terminal.draw(|f| {
            app.render(f, f.area());
        }).unwrap();

        let buffer = terminal.backend().buffer();

        // Assert specific cells contain expected content
        assert_eq!(buffer.get(0, 0).symbol(), "┌");

        // Or snapshot testing
        insta::assert_snapshot!(buffer);
    }
}
```

### Integration Testing with Mock Events

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_navigation() {
        let mut app = App::new();
        let mut state = AppState::default();

        // Simulate key presses
        app.handle_key(KeyCode::Down, &mut state).unwrap();
        assert_eq!(state.selected_index, 1);

        app.handle_key(KeyCode::Down, &mut state).unwrap();
        assert_eq!(state.selected_index, 2);

        app.handle_key(KeyCode::Up, &mut state).unwrap();
        assert_eq!(state.selected_index, 1);
    }
}
```

---

## Common Pitfalls and Solutions

### 1. Forgetting to Capture Builder Method Results

**Problem:**
```rust
let text = Text::raw("wrong");
text.centered(); // NO EFFECT - result discarded!
```

**Solution:**
```rust
let text = Text::raw("right").centered();
// Or
let mut text = Text::raw("right");
text = text.centered();
```

### 2. Buffer Overflow Panics

**Problem:** Rendering outside available area causes panic.

**Solution:** Always check area size before rendering:
```rust
if area.width >= min_width && area.height >= min_height {
    frame.render_widget(widget, area);
} else {
    // Render fallback or skip
}
```

### 3. Terminal State Not Restored

**Problem:** Terminal left in raw mode after crash.

**Solution:** Always use panic hooks and Drop implementation:
```rust
impl Drop for App {
    fn drop(&mut self) {
        let _ = restore_terminal();
    }
}
```

### 4. Event Loop Blocking

**Problem:** Long operations block rendering.

**Solution:** Offload to background threads:
```rust
let (tx, rx) = crossbeam_channel::unbounded();

// Background thread
thread::spawn(move || {
    let result = expensive_operation();
    tx.send(result).unwrap();
});

// TUI loop polls for results
while let Ok(result) = rx.try_recv() {
    app.update_with_result(result);
}
```

### 5. Unicode Handling in User Input

**Problem:** String indexing breaks on multibyte characters.

**Solution:** Use character indices:
```rust
// Wrong - byte index
self.input.remove(cursor_position);

// Right - character index
let char_indices: Vec<_> = self.input.char_indices().collect();
if cursor_position > 0 && cursor_position <= char_indices.len() {
    let byte_idx = char_indices[cursor_position - 1].0;
    self.input.remove(byte_idx);
}
```

---

## Example: Complete Multi-Pane Application

```rust
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
};
use std::{io, time::Duration};

struct App {
    input: String,
    messages: Vec<String>,
    selected: usize,
}

impl App {
    fn new() -> Self {
        Self {
            input: String::new(),
            messages: vec![],
            selected: 0,
        }
    }

    fn run(&mut self) -> io::Result<()> {
        let mut terminal = setup_terminal()?;

        loop {
            terminal.draw(|f| self.draw(f))?;

            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Char(c) => self.input.push(c),
                        KeyCode::Backspace => { self.input.pop(); }
                        KeyCode::Enter => {
                            self.messages.push(self.input.clone());
                            self.input.clear();
                        }
                        KeyCode::Down => {
                            if !self.messages.is_empty() {
                                self.selected = (self.selected + 1) % self.messages.len();
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        restore_terminal(terminal)?;
        Ok(())
    }

    fn draw(&self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(5),
                Constraint::Length(3),
            ])
            .split(f.area());

        // Header
        let header = Paragraph::new("Multi-Pane TUI Example")
            .block(Block::default().borders(Borders::ALL))
            .style(Style::default().fg(Color::Yellow));
        f.render_widget(header, chunks[0]);

        // Main area split horizontally
        let main = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[1]);

        // Messages list
        let items: Vec<ListItem> = self.messages
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let style = if i == self.selected {
                    Style::default().bg(Color::DarkGray)
                } else {
                    Style::default()
                };
                ListItem::new(m.as_str()).style(style)
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Messages"));
        f.render_widget(list, main[0]);

        // Info panel
        let info = Paragraph::new(format!(
            "Total messages: {}\nSelected: {}",
            self.messages.len(),
            self.selected
        ))
        .block(Block::default().borders(Borders::ALL).title("Info"));
        f.render_widget(info, main[1]);

        // Input
        let input = Paragraph::new(self.input.as_str())
            .block(Block::default().borders(Borders::ALL).title("Input"));
        f.render_widget(input, chunks[2]);
    }
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

fn restore_terminal(
    mut terminal: Terminal<CrosstermBackend<io::Stdout>>
) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn main() -> io::Result<()> {
    let mut app = App::new();
    app.run()
}
```

---

This reference provides comprehensive coverage of ratatui's capabilities. For the latest updates and additional examples, always consult the official documentation at https://ratatui.rs/.
