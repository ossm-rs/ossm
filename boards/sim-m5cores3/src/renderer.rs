/// Ratatui-powered renderer for the CoreS3 display via mousefood.
/// 
extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String as AllocString;
use core::fmt::Write;
use embedded_graphics::pixelcolor::{Rgb565, Rgb888, RgbColor};
use mousefood::fonts;
use mousefood::{ColorTheme, EmbeddedBackend, EmbeddedBackendConfig, TerminalAlignment};
use ratatui::Terminal;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::display::Display;

const CARRIAGE: &str = "[████]";

#[derive(Clone)]
pub struct FrameState {
    pub position: f64,
    pub depth: f64,
    pub stroke: f64,
    pub velocity: f64,
    pub sensation: f64,
    pub fps: u32,
    pub state: &'static str,
}


pub type OssmTerminal<'a> = Terminal<EmbeddedBackend<'a, Display, Rgb565>>;

pub fn create_terminal(display: &mut Display) -> OssmTerminal<'_> {
    let config = EmbeddedBackendConfig {
        font_regular: fonts::MONO_6X13,
        font_bold: Some(fonts::MONO_6X13_BOLD),
        font_italic: Some(fonts::MONO_6X13_ITALIC),
        flush_callback: Box::new(|_| {}),
        vertical_alignment: TerminalAlignment::Start,
        horizontal_alignment: TerminalAlignment::Start,
        color_theme: ColorTheme {
            foreground: Rgb888::GREEN,
            ..ColorTheme::ansi()
        },
    };
    let backend = EmbeddedBackend::new(display, config);
    let mut terminal = Terminal::new(backend).expect("terminal init");
    let _ = terminal.clear();
    terminal
}

pub fn render_ui(frame: &mut ratatui::Frame, state: &FrameState) {
    let area = frame.area();

    let block = Block::default()
        .borders(Borders::ALL)
        .title_alignment(Alignment::Center)
        .title(" OSSM Simulator ")
        .padding(ratatui::widgets::Padding::uniform(1));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Vertical layout: FPS top, rail centred, details bottom
    let rows = Layout::vertical([
        Constraint::Length(1), // 0: FPS
        Constraint::Fill(1),   // 1: flex space
        Constraint::Length(1), // 2: rail
        Constraint::Fill(1),   // 3: flex space
        Constraint::Length(1), // 4: pattern
        Constraint::Length(1), // 5: state
        Constraint::Length(1), // 6: depth
        Constraint::Length(1), // 7: stroke
        Constraint::Length(1), // 8: speed
        Constraint::Length(1), // 9: sensation
    ])
    .split(inner);

    // FPS counter
    let mut fps_str = AllocString::with_capacity(8);
    let _ = write!(fps_str, "{} fps", state.fps);
    frame.render_widget(Paragraph::new(fps_str).alignment(Alignment::Right), rows[0]);

    // OSSM Sim
    frame.render_widget(
        Paragraph::new(build_rail(state.position, inner.width as usize)),
        rows[2],
    );

    // Details
    frame.render_widget(Paragraph::new("Pattern: Deeper"), rows[4]);

    let mut state_line = AllocString::with_capacity(16);
    state_line.push_str("State:  ");
    state_line.push_str(state.state);
    frame.render_widget(Paragraph::new(state_line), rows[5]);
    render_gauge(frame, rows[6], "Depth:  ", state.depth, 0.0, 1.0);
    render_gauge(frame, rows[7], "Stroke: ", state.stroke, 0.0, 1.0);
    render_gauge(frame, rows[8], "Speed:  ", state.velocity, 0.0, 1.0);
    render_gauge(frame, rows[9], "Sense:  ", state.sensation, -1.0, 1.0);
}

fn build_rail(position_frac: f64, width: usize) -> AllocString {
    let carriage_len = CARRIAGE.chars().count();
    let track_len = width.saturating_sub(carriage_len);
    let pos = libm::round(position_frac * track_len as f64) as usize;
    let pos = pos.min(track_len);

    let mut s = AllocString::with_capacity(width);
    for _ in 0..pos {
        s.push('=');
    }
    s.push_str(CARRIAGE);
    for _ in (pos + carriage_len)..width {
        s.push('=');
    }
    s
}

fn render_gauge(frame: &mut ratatui::Frame, area: Rect, label: &str, value: f64, min: f64, max: f64) {
    let bipolar = min < 0.0 && max > 0.0;

    let cols = Layout::horizontal([
        Constraint::Length(8), // label
        Constraint::Fill(1),   // bar
        Constraint::Length(5), // value (e.g. "-1.00")
    ])
    .split(area);

    frame.render_widget(Paragraph::new(label), cols[0]);

    let bar_width = cols[1].width as usize;
    if bipolar {
        frame.render_widget(Paragraph::new(build_bipolar_bar(value, bar_width)), cols[1]);
    } else {
        let normalised = if max > min { (value - min) / (max - min) } else { 0.0 };
        frame.render_widget(Paragraph::new(build_bar(normalised, bar_width)), cols[1]);
    }

    let mut v = AllocString::with_capacity(7);
    if bipolar {
        let _ = write!(v, "{:>5.2}", value);
    } else {
        let _ = write!(v, "{:.2}", value);
    }
    frame.render_widget(Paragraph::new(v).alignment(Alignment::Right), cols[2]);
}

fn build_bar(value: f64, width: usize) -> AllocString {
    let inner = width.saturating_sub(2);
    let filled = libm::round(value * inner as f64) as usize;
    let filled = filled.min(inner);

    let mut s = AllocString::with_capacity(width);
    s.push('[');
    for _ in 0..filled.saturating_sub(1) {
        s.push('=');
    }
    if filled > 0 {
        s.push('>');
    }
    for _ in filled..inner {
        s.push(' ');
    }
    s.push(']');
    s
}

fn build_bipolar_bar(value: f64, width: usize) -> AllocString {
    let inner = width.saturating_sub(2);
    let mid = inner / 2;
    let clamped = value.clamp(-1.0, 1.0);
    // How many cells the bar extends from centre
    let extent = libm::round(libm::fabs(clamped) * mid as f64) as usize;
    let extent = extent.min(mid);

    let mut s = AllocString::with_capacity(width);
    s.push('[');
    if clamped < 0.0 && extent > 0 {
        // Negative: arrow grows leftward from centre  [ <====|     ]
        let start = mid - extent;
        for i in 0..inner {
            if i < start {
                s.push(' ');
            } else if i == start {
                s.push('<');
            } else if i < mid {
                s.push('=');
            } else if i == mid {
                s.push('|');
            } else {
                s.push(' ');
            }
        }
    } else if clamped > 0.0 && extent > 0 {
        // Positive: arrow grows rightward from centre [     |====> ]
        let end = mid + extent;
        for i in 0..inner {
            if i < mid {
                s.push(' ');
            } else if i == mid {
                s.push('|');
            } else if i < end {
                s.push('=');
            } else if i == end {
                s.push('>');
            } else {
                s.push(' ');
            }
        }
    } else {
        // Zero: just the centre marker [     |     ]
        for i in 0..inner {
            if i == mid {
                s.push('|');
            } else {
                s.push(' ');
            }
        }
    }
    s.push(']');
    s
}

