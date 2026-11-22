//! The full-screen app, one dashboard: the recordings, the selected recording (summary,
//! transcript, mind map, actions, jobs), the pipeline and the device pull. This file holds the
//! state and the keys; `draw` draws it. Reads and writes only through the use cases `App`
//! already exposes (the web `Backend` and `pull`).
mod draw;

use super::theme;
use crate::commands::pull::{Transfer, pull};
use crate::config::Config;
use crate::container::{self, App};
use liberarr_core::device::ports::KeyStore;
use liberarr_core::jobs::{JobInfo, JobQueue, Rerun, STEPS, rerun, run_job_once};
use liberarr_core::recordings::{RecordingItem, RecordingStatus};
use liberarr_core::summaries::{MeetingSummary, SummaryRepo};
use liberarr_core::transcription::Transcript;
use liberarr_web::Backend;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::widgets::ListState;
use rust_i18n::t;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tachyonfx::EffectManager;
use tokio::runtime::Handle;

const IDLE: Duration = Duration::from_secs(2);
/// The list and the selected recording are reloaded this often.
const REFRESH: Duration = Duration::from_secs(1);
const TOAST: Duration = Duration::from_secs(5);
/// The recordings and the preview are side by side from this width; below it, one at a time.
const WIDE: u16 = 100;

pub enum Exit {
    Quit,
    Settings,
}

type AnyError = Box<dyn std::error::Error>;

#[derive(Clone)]
enum Phase {
    Idle,
    Searching,
    Downloading(Transfer),
    /// The outcome lines, `true` when something failed, and when the pull ended.
    Finished(Vec<String>, bool, Instant),
}

struct PullState {
    running: bool,
    phase: Phase,
    downloaded: usize,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Summary,
    Transcript,
    Mindmap,
    Actions,
    Jobs,
}

const TABS: [Tab; 5] = [
    Tab::Summary,
    Tab::Transcript,
    Tab::Mindmap,
    Tab::Actions,
    Tab::Jobs,
];

/// The pane that takes the arrow keys. Below `WIDE` columns it is also the only one shown.
#[derive(PartialEq, Clone, Copy)]
enum Focus {
    List,
    Preview,
}

/// The selected recording, shown in the preview.
struct Detail {
    id: i64,
    transcript: Option<Transcript>,
    summary: Option<MeetingSummary>,
    jobs: Vec<JobInfo>,
}

/// One running effect of each kind: a new one replaces the one still playing.
/// `Default` only because `EffectManager::default` asks for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
enum Fx {
    #[default]
    Reveal,
    Preview,
    Finished,
}

/// Where the last frame drew what a click can hit. Empty when not on screen.
#[derive(Default)]
struct Areas {
    list: Rect,
    /// The rows of the recordings, under the search bar.
    rows: Rect,
    preview: Rect,
    tabs: Vec<Rect>,
}

struct Ui {
    app: Arc<App>,
    rt: Handle,
    items: Vec<RecordingItem>,
    list: ListState,
    filter: String,
    filtering: bool,
    focus: Focus,
    detail: Option<Detail>,
    tab: usize,
    scroll: u16,
    action: usize,
    /// Step menu of the rerun picker, open over the dashboard.
    picker: Option<usize>,
    help: bool,
    toast: Option<(String, Style, Instant)>,
    pull: Arc<Mutex<PullState>>,
    worker_error: Arc<Mutex<Option<String>>>,
    /// A device is linked (read once, at start).
    device: bool,
    last_pull: Option<jiff::Timestamp>,
    open_actions: usize,
    fx: EffectManager<Fx>,
    /// Effects to start at the next frame: the whole screen, the preview content, the row of
    /// the recording that just finished.
    reveal: bool,
    swap: bool,
    finished: Option<i64>,
    /// When the preview content last changed.
    swapped: Instant,
    /// Set by the last frame: the two panes are side by side; rows of the preview content.
    wide: bool,
    page: u16,
    areas: Areas,
    /// The mouse is captured; off, the terminal selects text.
    mouse: bool,
    refreshed: Instant,
}

/// Runs the app until the owner quits or opens the settings. The job worker runs in-process;
/// `serve` may run at the same time (claims are atomic, see `Db::reset_stale`).
pub fn run(term: &mut DefaultTerminal, cfg: Config) -> Result<Exit, AnyError> {
    let rt = tokio::runtime::Runtime::new()?;
    let warnings = container::warnings(&cfg);
    let app = Arc::new(App::open(cfg)?);
    rt.block_on(app.db.reset_stale())?;
    let mut ui = Ui::new(app, rt.handle().clone());
    let worker = {
        let (app, err) = (Arc::clone(&ui.app), Arc::clone(&ui.worker_error));
        rt.spawn(async move {
            loop {
                match run_job_once(&app.db, &*app).await {
                    Ok(true) => {}
                    Ok(false) => tokio::time::sleep(IDLE).await,
                    Err(e) => {
                        *lock(&err) = Some(e.to_string());
                        tokio::time::sleep(IDLE).await;
                    }
                }
            }
        })
    };
    if !warnings.is_empty() {
        ui.toast(warnings.join(" "), theme::warn());
    }
    ui.reload()?;
    capture(true)?;
    let exit = ui.event_loop(term);
    worker.abort();
    // The setup that may follow takes no mouse. On error, `Screen` turns it off.
    let _ = capture(false);
    exit
}

fn capture(on: bool) -> std::io::Result<()> {
    if on {
        execute!(std::io::stdout(), EnableMouseCapture)
    } else {
        execute!(std::io::stdout(), DisableMouseCapture)
    }
}

fn key(ev: &Event) -> Option<KeyEvent> {
    match ev {
        Event::Key(k) if k.kind != KeyEventKind::Release => Some(*k),
        _ => None,
    }
}

impl Ui {
    fn new(app: Arc<App>, rt: Handle) -> Ui {
        Ui {
            // An unreadable key file shows as "no device"; a pull reports the reason.
            device: container::keys(&app.cfg)
                .load()
                .is_ok_and(|k| !k.is_empty()),
            app,
            rt,
            items: Vec::new(),
            list: ListState::default().with_selected(Some(0)),
            filter: String::new(),
            filtering: false,
            focus: Focus::List,
            detail: None,
            tab: 0,
            scroll: 0,
            action: 0,
            picker: None,
            help: false,
            toast: None,
            pull: Arc::new(Mutex::new(PullState {
                running: false,
                phase: Phase::Idle,
                downloaded: 0,
            })),
            worker_error: Arc::new(Mutex::new(None)),
            last_pull: None,
            open_actions: 0,
            fx: EffectManager::default(),
            reveal: true,
            swap: false,
            finished: None,
            swapped: Instant::now(),
            wide: false,
            page: 0,
            areas: Areas::default(),
            mouse: true,
            refreshed: Instant::now(),
        }
    }

    fn toast(&mut self, text: impl Into<String>, style: Style) {
        self.toast = Some((text.into(), style, Instant::now()));
    }

    fn visible(&self) -> Vec<&RecordingItem> {
        let query = fold(self.filter.trim());
        self.items.iter().filter(|i| matches(i, &query)).collect()
    }

    fn processing(&self) -> bool {
        self.items
            .iter()
            .any(|i| i.status == RecordingStatus::Processing)
    }

    /// Reloads the list, the selected recording and the last pull. The selection stays on the
    /// same recording when new ones arrive above it.
    fn reload(&mut self) -> Result<(), AnyError> {
        let items = self.rt.block_on(self.app.list())?;
        if let Some(done) = items.iter().find(|new| {
            new.status == RecordingStatus::Done
                && self
                    .items
                    .iter()
                    .any(|old| old.id == new.id && old.status == RecordingStatus::Processing)
        }) {
            self.finished = Some(done.id);
        }
        self.items = items;
        let selected = self.detail.as_ref().map(|d| d.id);
        let at = self.visible().iter().position(|i| Some(i.id) == selected);
        if at.is_some() {
            self.list.select(at);
        }
        self.sync(true)?;
        self.last_pull = self.rt.block_on(self.app.db.last_pull())?;
        self.open_actions = self.rt.block_on(SummaryRepo::open_actions(&self.app.db))? as usize;
        let error = lock(&self.worker_error).take();
        if let Some(e) = error {
            self.toast(
                t!("tui.worker_error", error = e).into_owned(),
                theme::error(),
            );
        }
        self.refreshed = Instant::now();
        Ok(())
    }

    /// Makes the preview show the selected recording: loaded when the selection changed (it
    /// then starts at the top) or when `reload` is set.
    fn sync(&mut self, reload: bool) -> Result<(), AnyError> {
        let visible = self.visible();
        let at = self
            .list
            .selected()
            .unwrap_or(0)
            .min(visible.len().saturating_sub(1));
        let id = visible.get(at).map(|i| i.id);
        self.list.select(Some(at));
        let Some(id) = id else {
            self.detail = None;
            self.focus = Focus::List;
            return Ok(());
        };
        let changed = self.detail.as_ref().is_none_or(|d| d.id != id);
        if changed {
            (self.scroll, self.action, self.swap) = (0, 0, true);
        }
        if changed || reload {
            self.detail = Some(Detail {
                id,
                jobs: self.rt.block_on(self.app.jobs(id))?,
                transcript: self.rt.block_on(self.app.transcript(id))?,
                summary: self.rt.block_on(self.app.summary(id))?,
            });
        }
        Ok(())
    }

    fn event_loop(&mut self, term: &mut DefaultTerminal) -> Result<Exit, AnyError> {
        let mut last = Instant::now();
        loop {
            if self.refreshed.elapsed() >= REFRESH {
                self.reload()?;
            }
            if self
                .toast
                .as_ref()
                .is_some_and(|(_, _, at)| at.elapsed() >= TOAST)
            {
                self.toast = None;
            }
            let elapsed = last.elapsed();
            last = Instant::now();
            term.draw(|f| self.draw(f, elapsed))?;
            // Fast frames while something moves, slower otherwise. Input is never blocked longer.
            let wait = if self.fx.is_running() {
                30
            } else if lock(&self.pull).running || self.processing() {
                50
            } else {
                250
            };
            if !event::poll(Duration::from_millis(wait))? {
                continue;
            }
            // Pointer moves and releases change nothing: they wait for the next event without a
            // new frame, or the frame would be drawn again on every move.
            let ev = loop {
                let ev = event::read()?;
                let noise = matches!(&ev, Event::Mouse(m) if matches!(m.kind,
                    MouseEventKind::Moved | MouseEventKind::Drag(_) | MouseEventKind::Up(_)));
                if !noise || !event::poll(Duration::from_millis(wait))? {
                    break ev;
                }
            };
            if let Event::Mouse(m) = ev {
                self.on_mouse(m)?;
            }
            let Some(k) = key(&ev) else { continue };
            if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
                return Ok(Exit::Quit);
            }
            if let Some(exit) = self.on_key(k)? {
                return Ok(exit);
            }
        }
    }

    fn on_key(&mut self, k: KeyEvent) -> Result<Option<Exit>, AnyError> {
        if self.help {
            self.help = false;
            return Ok(None);
        }
        if let Some(sel) = self.picker {
            match k.code {
                KeyCode::Up | KeyCode::Char('k') => self.picker = Some(sel.saturating_sub(1)),
                KeyCode::Down | KeyCode::Char('j') => {
                    self.picker = Some((sel + 1).min(STEPS.len() - 1));
                }
                KeyCode::Enter => {
                    self.picker = None;
                    self.rerun(STEPS[sel])?;
                }
                KeyCode::Esc | KeyCode::Char('q') => self.picker = None,
                _ => {}
            }
            return Ok(None);
        }
        if self.filtering {
            match k.code {
                KeyCode::Enter | KeyCode::Esc => self.filtering = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) => self.filter.push(c),
                _ => {}
            }
            self.list.select(Some(0));
            return self.sync(false).map(|()| None);
        }
        let on_actions = TABS[self.tab] == Tab::Actions;
        let page = self.page.max(1);
        match (k.code, self.focus) {
            (KeyCode::Char('q'), _) => return Ok(Some(Exit::Quit)),
            (KeyCode::Char('?'), _) => self.help = true,
            (KeyCode::Char('p'), _) => self.start_pull(),
            (KeyCode::Char('s'), _) => return Ok(self.settings()),
            (KeyCode::Char('r'), _) if self.detail.is_some() => self.picker = Some(0),
            (KeyCode::Char('/'), _) => (self.filtering, self.focus) = (true, Focus::List),
            (KeyCode::Char('m'), _) => {
                self.mouse = !self.mouse;
                capture(self.mouse)?;
                let note = if self.mouse {
                    "tui.mouse_on"
                } else {
                    "tui.mouse_off"
                };
                self.toast(t!(note).into_owned(), theme::ok());
            }
            (KeyCode::Right, _) => self.show_tab((self.tab + 1) % TABS.len()),
            (KeyCode::Left, _) => self.show_tab((self.tab + TABS.len() - 1) % TABS.len()),
            (KeyCode::Char(c @ '1'..='5'), _) => self.show_tab(c as usize - '1' as usize),
            (KeyCode::Enter | KeyCode::Tab | KeyCode::BackTab, Focus::List)
                if self.detail.is_some() =>
            {
                self.focus = Focus::Preview;
            }
            (
                KeyCode::Esc | KeyCode::Backspace | KeyCode::Tab | KeyCode::BackTab,
                Focus::Preview,
            ) => self.focus = Focus::List,
            (KeyCode::Esc, Focus::List) if !self.filter.is_empty() => {
                self.filter.clear();
                self.list.select(Some(0));
            }
            (KeyCode::Esc, Focus::List) => return Ok(Some(Exit::Quit)),
            (KeyCode::Up | KeyCode::Char('k'), Focus::List) => self.move_selection(-1),
            (KeyCode::Down | KeyCode::Char('j'), Focus::List) => self.move_selection(1),
            // Side by side, the page keys scroll the preview whichever pane has the focus.
            (KeyCode::PageUp, Focus::List) if !self.wide => self.move_selection(-10),
            (KeyCode::PageDown, Focus::List) if !self.wide => self.move_selection(10),
            (KeyCode::Char(' '), Focus::Preview) if on_actions => self.toggle_action(),
            (KeyCode::Up | KeyCode::Char('k'), Focus::Preview) => self.step_preview(-1),
            (KeyCode::Down | KeyCode::Char('j'), Focus::Preview) => self.step_preview(1),
            (KeyCode::PageUp, _) => self.scroll = self.scroll.saturating_sub(page),
            (KeyCode::PageDown, _) => self.scroll = self.scroll.saturating_add(page),
            _ => {}
        }
        self.sync(false)?;
        Ok(None)
    }

    /// A click picks a recording, a tab or a pane; a click on the selected recording opens it,
    /// as Enter does. The wheel moves in the pane under the pointer, as the arrow keys do. An
    /// overlay closes on a click, as on a key.
    fn on_mouse(&mut self, m: MouseEvent) -> Result<(), AnyError> {
        let at = Position::new(m.column, m.row);
        let click = m.kind == MouseEventKind::Down(MouseButton::Left);
        if self.help || self.picker.is_some() {
            if click {
                (self.help, self.picker) = (false, None);
            }
            return Ok(());
        }
        let by = match m.kind {
            MouseEventKind::ScrollUp => -1,
            MouseEventKind::ScrollDown => 1,
            _ => 0,
        };
        let a = &self.areas;
        if self.areas.list.contains(at) {
            if by != 0 {
                self.move_selection(by);
            } else if click {
                self.focus = Focus::List;
                let row = a
                    .rows
                    .contains(at)
                    .then(|| self.list.offset() + usize::from(at.y - a.rows.y));
                match row.filter(|&r| r < self.visible().len()) {
                    Some(r) if self.list.selected() == Some(r) => self.focus = Focus::Preview,
                    Some(r) => self.list.select(Some(r)),
                    None => {}
                }
            }
        } else if a.preview.contains(at) {
            if by != 0 {
                self.step_preview(by);
            } else if click {
                self.focus = Focus::Preview;
                if let Some(tab) = a.tabs.iter().position(|r| r.contains(at)) {
                    self.show_tab(tab);
                }
            }
        }
        self.sync(false)
    }

    /// One step down (or up) in the preview: the cursor of the actions while it shows, the
    /// scroll otherwise. The frame clamps the scroll to the content.
    fn step_preview(&mut self, by: isize) {
        let actions = self
            .detail
            .as_ref()
            .and_then(|d| d.summary.as_ref())
            .map_or(0, |s| s.actions.len());
        if self.focus == Focus::Preview && TABS[self.tab] == Tab::Actions {
            let at = self.action.saturating_add_signed(by);
            self.action = at.min(actions.saturating_sub(1));
        } else if by < 0 {
            self.scroll = self.scroll.saturating_sub(1);
        } else {
            self.scroll = self.scroll.saturating_add(1);
        }
    }

    fn show_tab(&mut self, tab: usize) {
        if tab != self.tab {
            (self.tab, self.scroll, self.swap) = (tab, 0, true);
        }
    }

    fn move_selection(&mut self, by: isize) {
        let last = self.visible().len().saturating_sub(1);
        let at = self.list.selected().unwrap_or(0).saturating_add_signed(by);
        self.list.select(Some(at.min(last)));
    }

    fn toggle_action(&mut self) {
        let Some(d) = &self.detail else { return };
        let (id, index) = (d.id, self.action);
        let Some(a) = d
            .summary
            .as_ref()
            .and_then(|s| s.actions.get(index))
            .cloned()
        else {
            return;
        };
        let updated = self
            .rt
            .block_on(self.app.toggle_action(id, index, &a.action, !a.done));
        match updated {
            Ok(s) => {
                if let Some(d) = &mut self.detail {
                    d.summary = Some(s);
                }
            }
            Err(e) => self.toast(e.to_string(), theme::error()),
        }
    }

    /// Same guard as the web API: a second run while the pipeline runs would call the AI twice.
    fn rerun(&mut self, step: &str) -> Result<(), AnyError> {
        let Some(id) = self.detail.as_ref().map(|d| d.id) else {
            return Ok(());
        };
        if self.rt.block_on(rerun(&self.app.db, id, Some(step)))? == Rerun::Busy {
            self.toast(t!("tui.rerun_busy").into_owned(), theme::warn());
        } else {
            self.toast(
                t!("tui.rerun_queued", step = step).into_owned(),
                theme::ok(),
            );
        }
        self.reload()
    }

    /// The settings reopen the app with a new configuration: not while work is in progress.
    fn settings(&mut self) -> Option<Exit> {
        if lock(&self.pull).running || self.processing() {
            self.toast(t!("tui.settings_busy").into_owned(), theme::warn());
            return None;
        }
        Some(Exit::Settings)
    }

    /// Pulls on its own thread (Bluetooth can take minutes); the worker processes what arrives.
    fn start_pull(&mut self) {
        {
            let mut s = lock(&self.pull);
            if s.running {
                return;
            }
            *s = PullState {
                running: true,
                phase: Phase::Searching,
                downloaded: 0,
            };
        }
        let (app, rt, state) = (
            Arc::clone(&self.app),
            self.rt.clone(),
            Arc::clone(&self.pull),
        );
        std::thread::spawn(move || {
            let progress = Arc::clone(&state);
            let on = Arc::new(move |t: Option<&Transfer>| {
                let mut s = lock(&progress);
                match t {
                    Some(t) => s.phase = Phase::Downloading(t.clone()),
                    None => s.downloaded += 1,
                }
            });
            let outcome = rt.block_on(pull(&app, on));
            let (lines, failed) = match outcome {
                Ok(None) => (vec![t!("pull.no_device_linked").into_owned()], true),
                Ok(Some(r)) if r.no_device => {
                    (vec![t!("pull.no_device_in_range").into_owned()], true)
                }
                Ok(Some(r)) => {
                    let mut lines = vec![
                        t!(
                            "pull.summary",
                            pulled = r.pulled.len(),
                            skipped = r.skipped_recording.len(),
                            failed = r.failed.len()
                        )
                        .into_owned(),
                    ];
                    lines.extend(r.failed.iter().map(|(session, why)| {
                        t!("pull.failed_session", session = session, why = why).into_owned()
                    }));
                    lines.extend(r.device_errors.iter().map(|(tail, why)| {
                        t!("pull.device_error", tail = tail, why = why).into_owned()
                    }));
                    let failed = !r.failed.is_empty() || !r.device_errors.is_empty();
                    (lines, failed)
                }
                Err(e) => (vec![e.to_string()], true),
            };
            let mut s = lock(&state);
            s.running = false;
            s.phase = Phase::Finished(lines, failed, Instant::now());
        });
    }
}

/// Lowercase, accents removed: "Réunion" and "reunion" compare equal.
// shortcut: only the accented Latin letters of French and nearby languages are folded; other scripts compare as they are. Upgrade to Unicode normalization if users search other languages.
fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.to_lowercase().chars() {
        match c {
            'à' | 'â' | 'ä' => out.push('a'),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'î' | 'ï' => out.push('i'),
            'ô' | 'ö' => out.push('o'),
            'ù' | 'û' | 'ü' => out.push('u'),
            'ç' => out.push('c'),
            'ÿ' => out.push('y'),
            'ñ' => out.push('n'),
            'œ' => out.push_str("oe"),
            'æ' => out.push_str("ae"),
            // Combining accents: "é" typed as "e" + U+0301 (macOS file names) folds too.
            '\u{300}'..='\u{36f}' => {}
            c => out.push(c),
        }
    }
    out
}

/// A recording matches on the name the list shows for it, untitled ones included.
/// `query` is already folded.
fn matches(i: &RecordingItem, query: &str) -> bool {
    fold(&draw::title(i)).contains(query)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Profile;
    use liberarr_core::ai::Provider;
    use liberarr_core::lang::Language;
    use liberarr_core::recordings::{NewRecording, RecordingRepo, Source};
    use liberarr_core::summaries::{Action, SummaryRepo};
    use liberarr_core::transcription::{Segment, TranscriptRepo};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    fn profile() -> Profile {
        Profile {
            provider: Provider::Compatible,
            base_url: String::new(),
            api_key: String::new(),
            model: "m".into(),
            routing_only: Vec::new(),
            program: None,
        }
    }

    /// A draw that panics is a crash for the user: every pane, tab and overlay is drawn at a
    /// wide, a medium, a narrow, the minimum and a too-small size. At 140×40 the list, the
    /// summary, the pipeline and the pull progress are on screen together.
    #[test]
    fn dashboard_draws_at_every_size() {
        let home = std::env::temp_dir().join(format!(
            "liberarr-tui-{}-{}",
            std::process::id(),
            jiff::Timestamp::now().as_nanosecond()
        ));
        let rt = tokio::runtime::Runtime::new().unwrap();
        let app = Arc::new(
            App::open(Config {
                home: home.clone(),
                secret_key: "k".repeat(32),
                api_key: String::new(),
                admin_password: String::new(),
                transcribe: profile(),
                llm: Some(profile()),
                vocabulary: Vec::new(),
                obsidian: home.join("notes"),
                content_language: Language::En,
                content_language_set: false,
                cookie_secure: false,
                claude_oauth_token: None,
            })
            .unwrap(),
        );
        let mut ui = Ui::new(Arc::clone(&app), rt.handle().clone());
        // Two frames: the one that starts the effects, then one after they ended.
        let draw = |ui: &mut Ui, w, h| {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            for elapsed in [40, 1000] {
                term.draw(|f| ui.draw(f, Duration::from_millis(elapsed)))
                    .unwrap();
            }
            term.backend().buffer().clone()
        };
        let text = |buf: &Buffer| buf.content().iter().map(|c| c.symbol()).collect::<String>();
        // The top left cell of `s` on screen.
        let find = |buf: &Buffer, s: &str| {
            let (w, h) = (buf.area.width, buf.area.height);
            (0..h)
                .flat_map(|y| (0..w).map(move |x| (x, y)))
                .find(|&(x, y)| {
                    (x..w)
                        .map(|x| buf[(x, y)].symbol())
                        .collect::<String>()
                        .starts_with(s)
                })
                .unwrap_or_else(|| panic!("{s} is not on screen"))
        };
        let click = |ui: &mut Ui, (column, row): (u16, u16), kind| {
            ui.on_mouse(MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            })
            .unwrap();
        };
        let left = MouseEventKind::Down(MouseButton::Left);
        // No recording yet.
        ui.reload().unwrap();
        draw(&mut ui, 140, 40);
        draw(&mut ui, 60, 16);

        let add = |title: &str, at: &str| {
            rt.block_on(app.db.add(NewRecording {
                title: Some(title.into()),
                source: Source::File,
                source_ref: title.into(),
                audio_path: "a.ogg".into(),
                duration_s: Some(754.0),
                created_at: at.parse().unwrap(),
            }))
            .unwrap()
        };
        let done = add("Budget review", "2026-03-02T10:00:00Z");
        rt.block_on(TranscriptRepo::save(
            &app.db,
            done,
            &Transcript {
                engine: "e".into(),
                language: None,
                text: "Bonjour, let's start.".into(),
                segments: vec![Segment {
                    start: 0.0,
                    end: 2.0,
                    text: "Bonjour, let's start.".into(),
                }],
                has_timestamps: true,
            },
        ))
        .unwrap();
        rt.block_on(SummaryRepo::save(
            &app.db,
            done,
            &MeetingSummary {
                title: "Budget review".into(),
                summary_md: "# Decisions\n- **Ship** on Friday\n".repeat(30),
                mindmap: "mindmap\n  root((Budget))\n    Costs".into(),
                actions: vec![
                    Action {
                        action: "Send the figures to the whole team before the next review"
                            .repeat(3),
                        owner: Some("Lina".into()),
                        due: None,
                        start: Some(61.0),
                        done: false,
                    };
                    12
                ],
            },
        ))
        .unwrap();
        let failed = add("Failed one", "2026-03-01T10:00:00Z");
        rt.block_on(app.db.enqueue(failed, "transcribe")).unwrap();
        let job = rt.block_on(app.db.claim_next()).unwrap().unwrap();
        rt.block_on(app.db.fail(&job, "provider: boom")).unwrap();
        ui.reload().unwrap();
        ui.toast(
            "A long notice that wraps in its box. ".repeat(4),
            theme::warn(),
        );
        *lock(&ui.pull) = PullState {
            running: true,
            phase: Phase::Downloading(Transfer {
                label: "2026-03-02 11:00".into(),
                done: 3_000_000,
                total: 10_000_000,
                speed: 210_000.0,
                eta: 8,
            }),
            downloaded: 1,
        };
        ui.finished = Some(done);

        let wide = draw(&mut ui, 140, 40);
        for shown in ["Budget review", "Ship on Friday", "summarize", "30%"] {
            assert!(
                text(&wide).contains(shown),
                "{shown} is not on the dashboard"
            );
        }

        // Clicks on the last recording, under it, on the border, on the first one.
        let (x, last) = find(&wide, "Failed one");
        click(&mut ui, (x, last), left);
        assert_eq!(ui.list.selected(), Some(1));
        for empty in [last + 1, last - 2] {
            click(&mut ui, (x, empty), left);
            assert_eq!(ui.list.selected(), Some(1));
        }
        assert!(ui.focus == Focus::List);
        // The list's left border, then the first row; a second click on it opens the preview.
        click(&mut ui, (0, last - 1), left);
        assert_eq!(ui.list.selected(), Some(1));
        click(&mut ui, (x, last - 1), left);
        assert_eq!(ui.list.selected(), Some(0));
        assert!(ui.focus == Focus::List);
        click(&mut ui, (x, last - 1), left);
        assert!(ui.focus == Focus::Preview);
        // The wheel over the list moves the selection, a click on a tab opens it.
        click(&mut ui, (x, last), MouseEventKind::ScrollDown);
        assert_eq!(ui.list.selected(), Some(1));
        click(&mut ui, (x, last), MouseEventKind::ScrollUp);
        let wide = draw(&mut ui, 140, 40);
        let (jobs, y) = find(&wide, "Jobs");
        click(&mut ui, (jobs, y), left);
        assert_eq!(ui.tab, 4);
        // The last column of the Summary tab, padding included.
        let (summary, y) = find(&wide, "Summary");
        click(&mut ui, (summary + 8, y), left);
        assert_eq!(ui.tab, 0);
        let (x, y) = find(&wide, "Ship on Friday");
        click(&mut ui, (x, y), MouseEventKind::ScrollDown);
        assert_eq!(ui.scroll, 1);
        // An overlay takes the click and closes.
        ui.help = true;
        click(&mut ui, (jobs, y), left);
        assert!(!ui.help && ui.tab == 0);
        ui.scroll = 0;
        for (w, h) in [(200, 60), (140, 40), (100, 30), (80, 24), (60, 16), (30, 8)] {
            for focus in [Focus::List, Focus::Preview] {
                for tab in 0..TABS.len() {
                    (ui.focus, ui.tab, ui.action, ui.scroll) = (focus, tab, 11, u16::MAX);
                    draw(&mut ui, w, h);
                }
            }
            (ui.help, ui.picker, ui.filtering) = (true, Some(1), true);
            draw(&mut ui, w, h);
            (ui.help, ui.picker, ui.filtering) = (false, None, false);
        }
        // The failed recording, then a search without a match, then the end of a pull.
        ui.move_selection(1);
        ui.sync(false).unwrap();
        draw(&mut ui, 100, 30);
        ui.filter = "no such title".into();
        ui.sync(false).unwrap();
        lock(&ui.pull).phase = Phase::Finished(
            vec![t!("pull.no_device_in_range").into_owned()],
            true,
            Instant::now(),
        );
        draw(&mut ui, 140, 40);
        draw(&mut ui, 60, 16);
        drop(ui);
        drop(app);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn search_ignores_accents_and_finds_untitled() {
        let item = |title: Option<&str>| RecordingItem {
            id: 1,
            title: title.map(Into::into),
            source: Source::File,
            duration_s: None,
            created_at: jiff::Timestamp::UNIX_EPOCH,
            status: RecordingStatus::Done,
            failed_step: None,
            no_speech: false,
        };
        assert!(matches(&item(Some("Réunion d'équipe")), &fold("reunion")));
        assert!(matches(&item(Some("Re\u{301}union")), &fold("reunion")));
        assert!(matches(&item(Some("Cœur")), &fold("coeur")));
        assert!(matches(&item(None), &fold(&t!("tui.untitled"))));
        assert!(!matches(&item(Some("Cœur")), &fold("reunion")));
    }
}
