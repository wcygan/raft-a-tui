---
name: ratatui-tui-builder
description: Build terminal user interfaces using ratatui - widgets, layouts, event handling, rendering, and application patterns. Use when creating TUIs, implementing terminal UIs, building interactive CLI apps, or working with ratatui/crossterm. Keywords: ratatui, TUI, terminal UI, crossterm, widgets, layout, event handling, immediate mode rendering
---

# Ratatui TUI Builder

Expert guidance for building terminal user interfaces with ratatui - a Rust crate for creating sophisticated TUI applications using immediate mode rendering.

## Core Concepts

### Immediate Mode Rendering

Ratatui uses **immediate mode rendering** where the UI is reconstructed every frame based on current application state:

```rust
loop {
    terminal.draw(|f| {
        // UI rebuilt every frame from current state
        if state.condition {
            f.render_widget(SomeWidget::new(), layout);
        } else {
            f.render_widget(AnotherWidget::new(), layout);
        }
    })?;
}
```

**Key Advantage**: UI logic directly mirrors application state without manual synchronization.

### Builder-Lite Pattern

All ratatui widgets use the builder-lite pattern for fluent configuration:

```rust
// Correct - chain methods or capture result
let text = Text::raw("hello").centered();
let paragraph = Paragraph::new("content")
    .block(Block::bordered())
    .style(Style::default().fg(Color::Yellow));

// WRONG - methods return new value, original unchanged
let text = Text::raw("wrong");
text.centered(); // Has no effect! Result discarded
```

**Critical**: Setter methods consume `self` and return modified instance. Use `#[must_use]` to catch this mistake.

## Application Architecture

### Standard App Structure

```rust
struct App {
    // User interaction state
    input: String,
    input_mode: InputMode,  // Normal vs Editing
    cursor_position: usize,

    // Application state
    state: AppState,
    data: BTreeMap<String, String>,

    // Display state
    history: Vec<String>,
    messages: VecDeque<String>,

    // Communication channels (if multi-threaded)
    state_rx: Receiver<StateUpdate>,
    cmd_tx: Sender<Command>,
}
```

### Event Loop Pattern (Non-blocking with Updates)

```rust
impl App {
    fn run(&mut self) -> io::Result<()> {
        let mut terminal = setup_terminal()?;

        loop {
            // 1. Process pending updates (non-blocking)
            while let Ok(update) = self.state_rx.try_recv() {
                self.apply_update(update);
            }

            // 2. Render current state
            terminal.draw(|frame| self.draw(frame))?;

            // 3. Handle input with timeout (enables responsive updates)
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    match self.handle_key_event(key) {
                        KeyResult::Quit => break,
                        KeyResult::Command(cmd) => self.cmd_tx.send(cmd)?,
                        KeyResult::Continue => {}
                    }
                }
            }
        }

        restore_terminal(terminal)?;
        Ok(())
    }
}
```

**Why `event::poll()` with timeout?**
- Allows checking for state updates even without user input
- Ensures UI refreshes at least every 50ms (20 FPS)
- Critical for real-time visualization

### Terminal Setup and Cleanup

```rust
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
```

**Critical**: Always restore terminal state on exit - implement Drop or use panic hooks.

## Layouts

### Coordinate System

Origin `(0, 0)` is **top-left corner**:
- x-coordinates: left → right
- y-coordinates: top → bottom

### Basic Layout Splitting

```rust
use ratatui::layout::{Layout, Direction, Constraint};

let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
        Constraint::Length(3),      // Fixed 3 rows
        Constraint::Min(10),        // At least 10 rows
        Constraint::Percentage(50), // 50% of remaining
    ])
    .split(frame.area());
```

### Constraint Types

- **Length(u16)**: Fixed size (rows/columns)
- **Percentage(u16)**: Relative to parent (e.g., 50%)
- **Ratio(u16, u16)**: Fractional allocation (e.g., 1/3)
- **Min(u16)**: Minimum size
- **Max(u16)**: Maximum size

**Important**: `Ratio` and `Percentage` are relative to parent size - may cause unexpected results when mixing with fixed constraints.

### Nested Layouts (Multi-Pane UI)

```rust
fn draw(&self, frame: &mut Frame) {
    // Vertical split: header, main, footer
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),   // Header
            Constraint::Min(10),     // Main content
            Constraint::Length(3),   // Footer
        ])
        .split(frame.area());

    // Horizontal split of main area
    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),  // Left pane
            Constraint::Percentage(50),  // Right pane
        ])
        .split(outer[1]);

    self.draw_header(frame, outer[0]);
    self.draw_left_pane(frame, main[0]);
    self.draw_right_pane(frame, main[1]);
    self.draw_footer(frame, outer[2]);
}
```

### Grid Layouts

```rust
// Modern approach (v0.30+)
let rows = Layout::vertical([Constraint::Length(5); 3])
    .split(area);

let cells: Vec<Rect> = rows
    .iter()
    .flat_map(|row| {
        Layout::horizontal([Constraint::Percentage(33); 3])
            .split(*row)
    })
    .collect();

// Render widgets into each cell
for (i, &cell) in cells.iter().enumerate() {
    frame.render_widget(
        Block::bordered().title(format!("Cell {}", i)),
        cell
    );
}
```

## Widgets

### Common Widget Types

- **Block**: Foundational widget with borders, titles, styling
- **Paragraph**: Styled and wrapped text content
- **List**: Selectable vertical items
- **Table**: Data in rows/columns with selection
- **BarChart**: Grouped or ungrouped bar graphs
- **Gauge**: Progress visualization
- **Tabs**: Tab bar interface
- **Canvas**: Custom shapes and drawings
- **Chart**: Line or scatter plots

### Widget vs StatefulWidget

**Widget trait**: Consumed during rendering (one-time use)
```rust
pub trait Widget {
    fn render(self, area: Rect, buf: &mut Buffer);
}
```

**StatefulWidget trait**: Maintains internal state (e.g., scroll position)
```rust
pub trait StatefulWidget {
    type State;
    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State);
}
```

**Modern variants** (v0.26+): `WidgetRef` and `StatefulWidgetRef` allow rendering by reference for reuse.

### Rendering Widgets

```rust
// Simple widget
frame.render_widget(my_widget, area);

// Stateful widget
frame.render_stateful_widget(list_widget, area, &mut list_state);
```

## Styling and Colors

### Color Types

```rust
use ratatui::style::Color;

// Named colors (16 standard terminal colors)
Color::Black, Color::Red, Color::Green, Color::Yellow,
Color::Blue, Color::Magenta, Color::Cyan, Color::Gray,
Color::DarkGray, Color::LightRed, etc.

// Indexed colors (256 palette: 0-255)
Color::Indexed(196)  // Bright red

// RGB colors
Color::Rgb(255, 128, 0)  // Orange

// Grayscale (indices 232-255)
Color::Indexed(240)  // Mid-gray
```

### Applying Styles

```rust
use ratatui::style::{Style, Modifier};

let paragraph = Paragraph::new("Hello")
    .style(Style::default()
        .fg(Color::Yellow)
        .bg(Color::Blue)
        .add_modifier(Modifier::BOLD));

// Using Stylize trait (shorthand)
use ratatui::style::Stylize;
let text = "Hello".yellow().bold();
```

### Text Hierarchy (Span → Line → Text)

```rust
use ratatui::text::{Span, Line, Text};

// Span: styled segment
let span = Span::styled("Error: ", Style::default().fg(Color::Red));

// Line: collection of spans (single row)
let line = Line::from(vec![
    Span::raw("Status: "),
    Span::styled("OK", Style::default().fg(Color::Green).bold()),
]);

// Text: collection of lines (multi-line)
let text = Text::from(vec![
    Line::from("First line"),
    Line::from(vec![
        Span::raw("Second line with "),
        Span::styled("color", Style::default().fg(Color::Cyan)),
    ]),
]);

// Alignment
let centered = Line::from("Title").centered();
```

## Event Handling

### Input Modes Pattern

```rust
#[derive(PartialEq)]
enum InputMode {
    Normal,   // Navigation/commands
    Editing,  // Text input
}

fn handle_key_event(&mut self, key: KeyEvent) -> KeyResult {
    match self.input_mode {
        InputMode::Normal => match key.code {
            KeyCode::Char('q') => KeyResult::Quit,
            KeyCode::Char('i') => {
                self.input_mode = InputMode::Editing;
                KeyResult::Continue
            }
            _ => KeyResult::Continue,
        },
        InputMode::Editing => match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                KeyResult::Continue
            }
            KeyCode::Enter => {
                let input = self.input.drain(..).collect::<String>();
                self.history.push(input.clone());
                KeyResult::Command(parse_command(&input))
            }
            KeyCode::Char(c) => {
                self.input.insert(self.cursor_position, c);
                self.cursor_position += 1;
                KeyResult::Continue
            }
            KeyCode::Backspace => {
                if self.cursor_position > 0 {
                    self.input.remove(self.cursor_position - 1);
                    self.cursor_position -= 1;
                }
                KeyResult::Continue
            }
            _ => KeyResult::Continue,
        },
    }
}
```

### Cursor Positioning

```rust
// Set visible cursor position during rendering
if self.input_mode == InputMode::Editing {
    frame.set_cursor_position((
        input_area.x + self.cursor_position as u16 + 1,
        input_area.y + 1,
    ));
}
```

### Event Handling Architectures

**1. Centralized** - Simple, single location, doesn't scale
**2. Message Passing** - Centralized capture, distributed handling via channels
**3. Distributed** - Each module handles own events, may duplicate code

**Recommendation**: Choose based on app complexity - centralized for simple apps, message passing for multi-threaded or modular apps.

## Component Architecture Pattern

For complex applications, organize as components with traits:

```rust
trait Component {
    fn init(&mut self) -> Result<()>;
    fn handle_events(&mut self, event: Event) -> Result<()>;
    fn handle_key_events(&mut self, key: KeyEvent) -> Result<()>;
    fn update(&mut self, action: Action) -> Result<()>;
    fn render(&mut self, frame: &mut Frame, area: Rect);
}
```

**Benefits**:
- Encapsulation: Components own private state
- Modularity: Decoupled event handling and rendering
- Scalability: Composition for complex apps

## Multi-Threading Pattern

For apps requiring background work (e.g., Raft consensus + TUI):

```rust
// Main thread: TUI event loop
let (state_tx, state_rx) = crossbeam_channel::unbounded();
let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();

// Spawn background thread
thread::spawn(move || {
    background_work(cmd_rx, state_tx);
});

// TUI loop
let mut app = App::new(state_rx, cmd_tx);
app.run()?;
```

**Key Points**:
- Use `try_recv()` to drain pending updates non-blocking
- Event loop timeout enables UI updates without user input
- Ring buffers (`VecDeque`) prevent unbounded memory growth

## Common Patterns and Best Practices

### Color-Coded Event Display

```rust
fn draw_logs(&self, frame: &mut Frame, area: Rect) {
    let logs: Vec<ListItem> = self.messages
        .iter()
        .map(|msg| {
            let style = match msg.event_type {
                EventType::Error => Style::default().fg(Color::Red),
                EventType::Warning => Style::default().fg(Color::Yellow),
                EventType::Info => Style::default().fg(Color::Green),
                EventType::Debug => Style::default().fg(Color::Gray),
            };
            ListItem::new(msg.text.as_str()).style(style)
        })
        .collect();

    let list = List::new(logs)
        .block(Block::default().borders(Borders::ALL).title("Events"));

    frame.render_widget(list, area);
}
```

### Ring Buffers for Logs

```rust
// Prevent unbounded growth
if self.messages.len() > 1000 {
    self.messages.pop_back();
}
self.messages.push_front(new_message);
```

### Panic Hooks for Terminal Cleanup

```rust
use std::panic;

fn install_panic_hook() {
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        let _ = restore_terminal();
        original_hook(panic_info);
    }));
}
```

## Testing TUI Applications

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tui_state_updates() {
        let (state_tx, state_rx) = crossbeam_channel::unbounded();
        let (cmd_tx, _cmd_rx) = crossbeam_channel::unbounded();

        // Send mock updates
        state_tx.send(StateUpdate::Data {
            key: "test".into(),
            value: "123".into(),
        }).unwrap();

        let mut app = App::new(state_rx, cmd_tx);

        // Process updates
        while let Ok(update) = app.state_rx.try_recv() {
            app.apply_update(update);
        }

        assert_eq!(app.data.get("test"), Some(&"123".to_string()));
    }
}
```

## Project Integration

When implementing TUIs in this project:

1. **Follow ROADMAP.md** - Check Phase 2 for TUI implementation tasks
2. **Use crossbeam channels** - For Raft thread ↔ TUI thread communication
3. **50ms poll timeout** - For ~20 FPS update rate
4. **Multi-pane layout** - 4-pane design per README requirements
5. **Color-coded events** - Visual feedback for different event types
6. **Non-blocking updates** - Use `try_recv()` to drain all pending state before rendering

## Reference Documentation

For deeper details, see REFERENCE.md which contains:
- Complete URL index of ratatui documentation
- Detailed widget examples and configurations
- Advanced layout techniques
- Async event handling patterns
- Production deployment recipes

## When to Use This Skill

Activate this skill when:
- Implementing terminal user interfaces
- Working with ratatui or crossterm
- Building interactive CLI applications
- Creating multi-pane TUI layouts
- Handling keyboard/mouse events in terminal apps
- Styling terminal output with colors
- Implementing immediate mode rendering patterns
