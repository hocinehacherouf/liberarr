//! `liberarr tui` and `init --ui tui`: a full-screen front-end over the same use cases as the CLI
//! and the web UI. No pipeline logic lives here. Secrets are only typed in masked fields and
//! never drawn.
mod app;
mod prompter;
mod text;

use crate::commands::{self, Outcome};
use crate::config;
use prompter::TuiPrompter;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use ratatui::crossterm::execute;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use std::process::ExitCode;
use std::sync::OnceLock;

/// Smallest usable window; below it a message asks to enlarge it.
const MIN_WIDTH: u16 = 60;
const MIN_HEIGHT: u16 = 16;

/// The terminal in full-screen raw mode, restored on drop (normal exit or error). `ratatui::init`
/// also installs a panic hook that restores it.
struct Screen(DefaultTerminal);

impl Screen {
    fn open() -> std::io::Result<Screen> {
        let term = ratatui::init();
        // A pasted API key arrives as one event instead of one key per character.
        execute!(std::io::stdout(), EnableBracketedPaste)?;
        Ok(Screen(term))
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = execute!(std::io::stdout(), DisableBracketedPaste);
        ratatui::restore();
    }
}

/// Colors are off with `NO_COLOR` or a dumb terminal.
fn colors() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
            && std::env::var("TERM").map_or(true, |t| t != "dumb")
    })
}

/// 24-bit colors only when the terminal says it draws them (`COLORTERM`).
fn truecolor() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        colors() && std::env::var("COLORTERM").is_ok_and(|v| v == "truecolor" || v == "24bit")
    })
}

/// Transitions are off without colors or with `LIBERARR_REDUCED_MOTION=1`.
fn motion() -> bool {
    colors() && std::env::var("LIBERARR_REDUCED_MOTION").map_or(true, |v| v != "1")
}

/// Named ANSI colors for text: they follow the terminal's palette, readable on dark and light
/// themes. The brand gradient is the one 24-bit accent.
mod theme {
    use super::{Color, Modifier, Span, Style, colors, truecolor};

    fn fg(c: Color) -> Style {
        if colors() {
            Style::new().fg(c)
        } else {
            Style::new()
        }
    }
    pub fn accent() -> Style {
        fg(Color::Cyan).add_modifier(Modifier::BOLD)
    }
    pub fn dim() -> Style {
        fg(Color::DarkGray)
    }
    pub fn ok() -> Style {
        fg(Color::Green)
    }
    pub fn warn() -> Style {
        fg(Color::Yellow)
    }
    pub fn error() -> Style {
        fg(Color::Red)
    }
    pub fn selected() -> Style {
        if colors() {
            Style::new()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().add_modifier(Modifier::REVERSED)
        }
    }
    pub fn bold() -> Style {
        Style::new().add_modifier(Modifier::BOLD)
    }
    /// The border of a pane: the accent when it has the focus.
    pub fn border(focused: bool) -> Style {
        if focused { accent() } else { dim() }
    }
    /// The brand color at `t` (0 to 1) of its cyan-to-violet gradient; the accent without
    /// 24-bit colors.
    pub fn brand(t: f32) -> Style {
        if !truecolor() {
            return accent();
        }
        let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t) as u8;
        Style::new()
            .fg(Color::Rgb(mix(0, 175), mix(190, 110), mix(215, 255)))
            .add_modifier(Modifier::BOLD)
    }
    /// `text` in the brand gradient, one color step per character.
    pub fn gradient(text: &str) -> Vec<Span<'static>> {
        let last = text.chars().count().saturating_sub(1).max(1) as f32;
        text.chars()
            .enumerate()
            .map(|(i, c)| Span::styled(c.to_string(), brand(i as f32 / last)))
            .collect()
    }
    /// The color effects fade from: the background of most terminals.
    pub const FADE: Color = Color::Black;
}

/// Shown on the setup's first screen and on an empty recordings list.
const LOGO: [&str; 3] = [
    "╻  ╻┏┓ ┏━╸┏━┓┏━┓┏━┓┏━┓",
    "┃  ┃┣┻┓┣╸ ┣┳┛┣━┫┣┳┛┣┳┛",
    "┗━╸╹┗━┛┗━╸╹┗╸╹ ╹╹┗╸╹┗╸",
];

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// The spinner frame for now: one frame every 80 ms.
fn spinner() -> char {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    let ms = START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis();
    SPINNER[(ms / 80) as usize % SPINNER.len()]
}

/// `init --ui tui`: the setup alone; the caller prints the result once the terminal is restored.
pub fn setup_only() -> Result<commands::init::Saved, Box<dyn std::error::Error>> {
    let mut screen = Screen::open()?;
    commands::init::apply(Some(&mut TuiPrompter::new(&mut screen.0)))
}

/// `liberarr tui`: the setup first when the configuration is missing or incomplete, then the app.
/// Settings (reconfigure) reopen the app with the new configuration.
pub fn run() -> Outcome {
    let home = config::home()?;
    let mut screen = Screen::open()?;
    let mut reconfigure = config::load(&home).is_err();
    loop {
        if reconfigure {
            // Without a working configuration there is no app to go back to.
            let usable = config::load(&home).is_ok();
            match commands::init::apply(Some(&mut TuiPrompter::new(&mut screen.0))) {
                Ok(saved) => prompter::done(&mut screen.0, &commands::init::report(&saved))?,
                // Cancelling the settings goes back to the app; cancelling the first setup quits.
                Err(e)
                    if usable
                        && e.downcast_ref::<std::io::Error>()
                            .is_some_and(|e| e.kind() == std::io::ErrorKind::Interrupted) => {}
                Err(e) => return Err(e),
            }
        }
        let cfg = config::load(&home)?;
        match app::run(&mut screen.0, cfg)? {
            app::Exit::Quit => return Ok(ExitCode::SUCCESS),
            app::Exit::Settings => reconfigure = true,
        }
    }
}
