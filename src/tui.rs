use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    symbols,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType},
    Frame, Terminal,
};
use std::collections::VecDeque;
use std::io;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Simple TUI application for displaying frequency shift data
pub struct SimpleTui {
    /// Frequency shift data points (cycle, value)
    freq_shift_data: VecDeque<(f64, f64)>,
    /// Maximum number of data points to keep
    max_data_points: usize,
    /// Current cycle number
    current_cycle: u32,
    /// Data receiver channel
    rx: mpsc::Receiver<f32>,
}

impl SimpleTui {
    /// Create a new SimpleTui instance
    pub fn new(rx: mpsc::Receiver<f32>) -> Self {
        Self {
            freq_shift_data: VecDeque::new(),
            max_data_points: 100, // Keep last 100 points
            current_cycle: 0,
            rx,
        }
    }

    /// Add a new frequency shift data point
    pub fn add_data_point(&mut self, freq_shift: f32) {
        self.current_cycle += 1;
        self.freq_shift_data
            .push_back((self.current_cycle as f64, freq_shift as f64));

        // Keep only the last N data points
        while self.freq_shift_data.len() > self.max_data_points {
            self.freq_shift_data.pop_front();
        }

        // Log data addition (using log crate instead of println)
        log::debug!("TUI: Added data point - Cycle: {}, Freq Shift: {:.3}, Total points: {}", 
                   self.current_cycle, freq_shift, self.freq_shift_data.len());
    }

    /// Check for new data from the receiver (non-blocking)
    pub fn update_data(&mut self) {
        while let Ok(freq_shift) = self.rx.try_recv() {
            self.add_data_point(freq_shift);
        }
    }

    /// Get the data for rendering the chart
    fn get_chart_data(&self) -> Vec<(f64, f64)> {
        self.freq_shift_data.iter().cloned().collect()
    }

    /// Get the bounds for the chart axes
    fn get_chart_bounds(&self) -> ([f64; 2], [f64; 2]) {
        if self.freq_shift_data.is_empty() {
            return ([0.0, 1.0], [-1.0, 1.0]);
        }

        let x_min = self
            .freq_shift_data
            .iter()
            .map(|(x, _)| *x)
            .fold(f64::INFINITY, f64::min);
        let x_max = self
            .freq_shift_data
            .iter()
            .map(|(x, _)| *x)
            .fold(f64::NEG_INFINITY, f64::max);

        let y_min = self
            .freq_shift_data
            .iter()
            .map(|(_, y)| *y)
            .fold(f64::INFINITY, f64::min);
        let y_max = self
            .freq_shift_data
            .iter()
            .map(|(_, y)| *y)
            .fold(f64::NEG_INFINITY, f64::max);

        // Add some padding to the bounds
        let x_padding = (x_max - x_min) * 0.05;
        let y_padding = (y_max - y_min) * 0.1;

        (
            [x_min - x_padding, x_max + x_padding],
            [y_min - y_padding, y_max + y_padding],
        )
    }
}

impl SimpleTui {
    /// Run the TUI application
    pub fn run<B: Backend>(
        mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let tick_rate = Duration::from_millis(100); // 10 FPS
        let mut last_tick = Instant::now();

        loop {
            // Check for new data
            self.update_data();

            // Draw the UI
            terminal.draw(|f| self.ui(f))?;

            // Handle events
            let timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));

            if crossterm::event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        _ => {}
                    }
                }
            }

            if last_tick.elapsed() >= tick_rate {
                last_tick = Instant::now();
            }
        }
    }

    /// Draw the user interface
    fn ui(&self, f: &mut Frame) {
        let size = f.size();

        // Create the layout
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(90), Constraint::Percentage(10)].as_ref())
            .split(size);

        // Draw the frequency shift chart
        self.draw_frequency_chart(f, chunks[0]);

        // Draw the status bar
        self.draw_status_bar(f, chunks[1]);
    }

    /// Draw the frequency shift chart
    fn draw_frequency_chart(&self, f: &mut Frame, area: Rect) {
        let data = self.get_chart_data();
        let (x_bounds, y_bounds) = self.get_chart_bounds();

        let datasets = vec![Dataset::default()
            .name("Frequency Shift")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Cyan))
            .data(&data)];

        let chart = Chart::new(datasets)
            .block(
                Block::default()
                    .title("Frequency Shift Over Time")
                    .borders(Borders::ALL),
            )
            .x_axis(
                Axis::default()
                    .title("Cycle")
                    .style(Style::default().fg(Color::Gray))
                    .bounds(x_bounds),
            )
            .y_axis(
                Axis::default()
                    .title("Frequency Shift (Hz)")
                    .style(Style::default().fg(Color::Gray))
                    .bounds(y_bounds),
            );

        f.render_widget(chart, area);
    }

    /// Draw the status bar
    fn draw_status_bar(&self, f: &mut Frame, area: Rect) {
        let status_text = format!(
            "Cycle: {} | Data Points: {} | Press 'q' to quit",
            self.current_cycle,
            self.freq_shift_data.len()
        );

        let status_block = Block::default()
            .title("Status")
            .borders(Borders::ALL)
            .title_style(Style::default().fg(Color::Yellow));

        let status_paragraph = ratatui::widgets::Paragraph::new(status_text)
            .block(status_block)
            .style(Style::default().fg(Color::White));

        f.render_widget(status_paragraph, area);
    }
}

/// Initialize the terminal for TUI mode
pub fn init_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>, Box<dyn std::error::Error>>
{
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Restore the terminal to normal mode
pub fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), Box<dyn std::error::Error>> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}