//! Drawing of the dashboard: the header tiles, the recordings and the preview (side by side,
//! or one at a time on a narrow window), the activity pane, the key hints and the overlays.
use super::{Areas, Focus, Fx, Phase, TABS, Tab, Ui, WIDE, lock};
use crate::commands::pull::{Transfer, mb};
use crate::tui::text::{markdown, mindmap};
use crate::tui::{LOGO, MIN_HEIGHT, MIN_WIDTH, hints, motion, pane, spinner, sweep, theme};
use liberarr_core::diarization::{speaker_for, speaker_label};
use liberarr_core::jobs::{JobInfo, JobStatus, STEPS, latest_by_step};
use liberarr_core::recordings::{RecordingItem, RecordingStatus};
use liberarr_core::transcription::timestamp;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Margin, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Clear, List, ListItem, Padding, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Tabs, Wrap,
};
use rust_i18n::t;
use std::time::{Duration, Instant};
use tachyonfx::fx;

/// How long the outcome of a pull stays open in the activity pane before it folds to one line.
const OUTCOME: Duration = Duration::from_secs(30);
const WRAP: Wrap = Wrap { trim: false };

fn local(t: jiff::Timestamp, format: &str) -> String {
    t.to_zoned(jiff::tz::TimeZone::system())
        .strftime(format)
        .to_string()
}

fn width(s: &str) -> usize {
    Line::raw(s).width()
}

/// `s` cut to `max` columns, with an ellipsis when it was longer.
fn fit(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        out.push(c);
        if width(&out) >= max {
            out.pop();
            break;
        }
    }
    out + "…"
}

fn dim(s: &str) -> Span<'static> {
    Span::styled(s.to_string(), theme::dim())
}

/// A scrollbar in the right border of a pane, when the content is longer than `rows`.
fn scrollbar(f: &mut Frame, rows: Rect, pane: Rect, hidden: usize, position: usize) {
    if hidden == 0 || rows.height == 0 {
        return;
    }
    let track = Rect::new(pane.right().saturating_sub(1), rows.y, 1, rows.height);
    f.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .track_style(theme::dim())
            .thumb_symbol("┃"),
        track,
        &mut ScrollbarState::new(hidden + 1).position(position),
    );
}

/// A box of at most `width`×`height` in the middle of `area`, cleared.
fn popup(f: &mut Frame, area: Rect, width: u16, height: u16) -> Rect {
    let [v] = Layout::vertical([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .areas(area);
    let [r] = Layout::horizontal([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(v);
    f.render_widget(Clear, r);
    r
}

fn status_mark(i: &RecordingItem) -> (String, Style) {
    match i.status {
        RecordingStatus::Done if i.no_speech => ("○".into(), theme::dim()),
        RecordingStatus::Done => ("●".into(), theme::ok()),
        RecordingStatus::Processing => (spinner().to_string(), theme::accent()),
        RecordingStatus::Failed => ("▲".into(), theme::error()),
    }
}

pub(super) fn title(i: &RecordingItem) -> String {
    i.title
        .clone()
        .unwrap_or_else(|| t!("tui.untitled").into_owned())
}

/// One recording in the list. Status and date first: a long title never pushes them off the
/// pane. The duration and the note (failed step, no speech) show when there is room.
fn row(i: &RecordingItem, max: usize) -> Line<'static> {
    let (mark, style) = status_mark(i);
    let note = match i.status {
        RecordingStatus::Failed => t!(
            "tui.failed_step",
            step = i.failed_step.as_deref().unwrap_or("?")
        )
        .into_owned(),
        RecordingStatus::Done if i.no_speech => t!("no_speech.marker").into_owned(),
        _ => String::new(),
    };
    let date = local(i.created_at, "%m-%d %H:%M");
    let mut room = max.saturating_sub(width(&date) + 4);
    let duration = i.duration_s.map(timestamp).filter(|_| room >= 26);
    if duration.is_some() {
        room -= 7;
    }
    let note = Some(note).filter(|n| !n.is_empty() && room >= 30 + width(n));
    if let Some(n) = &note {
        room -= width(n) + 2;
    }
    let title = fit(&title(i), room);
    let pad = " ".repeat(room.saturating_sub(width(&title)));
    let mut spans = vec![
        Span::styled(format!("{mark} "), style),
        dim(&format!("{date}  ")),
        Span::raw(title + &pad),
    ];
    if let Some(n) = note {
        spans.push(Span::styled(format!("  {n}"), style));
    }
    if let Some(d) = duration {
        spans.push(dim(&format!("  {d:>5}")));
    }
    Line::from(spans)
}

/// Where the pipeline stands, from the latest job of each step (so a failed optional step still
/// shows after the jobs that followed it). While a job is queued or running, the steps after it
/// wait. A step without a job counts as done when a later one has run.
fn stepper(jobs: &[JobInfo]) -> Line<'static> {
    let position = |step: &str| STEPS.iter().position(|s| *s == step);
    let latest = latest_by_step(jobs);
    let reached = latest.iter().filter_map(|j| position(&j.step)).max();
    let waiting_after = jobs
        .last()
        .filter(|j| matches!(j.status, JobStatus::Queued | JobStatus::Running))
        .and_then(|j| position(&j.step));
    let mut spans = Vec::new();
    for (n, step) in STEPS.iter().enumerate() {
        if n > 0 {
            spans.push(dim(" ── "));
        }
        let status = latest.iter().find(|j| j.step == *step).map(|j| j.status);
        let (mark, style) = match status {
            _ if waiting_after.is_some_and(|w| n > w) => ("·".to_string(), theme::dim()),
            Some(JobStatus::Done) => ("✓".to_string(), theme::ok()),
            Some(JobStatus::Running) => (spinner().to_string(), theme::accent()),
            Some(JobStatus::Queued) => ("○".to_string(), theme::accent()),
            Some(JobStatus::Failed) => ("✗".to_string(), theme::error()),
            // Recordings from before diarization existed never ran it: not a ✓.
            None if *step != "diarize" && reached.is_some_and(|r| n < r) => {
                ("✓".to_string(), theme::ok())
            }
            None => ("·".to_string(), theme::dim()),
        };
        spans.push(Span::styled(format!("{mark} {step}"), style));
    }
    Line::from(spans)
}

/// The download in progress: label, bar, then how much, how fast and how long.
fn transfer(tr: &Transfer, count: usize, max: usize) -> Line<'static> {
    let ratio = if tr.total == 0 {
        0.0
    } else {
        (tr.done as f64 / tr.total as f64).clamp(0.0, 1.0)
    };
    // `MM-DD HH:MM`: the year is not worth its columns here.
    let label = tr.label.get(5..).unwrap_or(&tr.label);
    let label = format!(
        "{} {} ",
        spinner(),
        t!("tui.pull_downloading", label = label, count = count)
    );
    let mut stats = format!(
        " {:.0}%  {:.1}/{:.1} MB",
        ratio * 100.0,
        mb(tr.done),
        mb(tr.total)
    );
    if max >= 90 {
        stats += &format!(
            "  {:.0} kB/s  {}:{:02}",
            tr.speed / 1000.0,
            tr.eta / 60,
            tr.eta % 60
        );
    }
    let bar = max
        .saturating_sub(width(&label) + width(&stats))
        .clamp(8, 40);
    let filled = (ratio * bar as f64).round() as usize;
    let mut spans = vec![Span::styled(label, theme::accent())];
    spans.extend((0..filled).map(|n| Span::styled("█", theme::brand(n as f32 / bar as f32))));
    spans.push(dim(&"░".repeat(bar - filled)));
    spans.push(Span::raw(stats));
    Line::from(spans)
}

impl Ui {
    pub(super) fn draw(&mut self, f: &mut Frame, elapsed: Duration) {
        let area = f.area();
        self.areas = Areas::default();
        if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
            f.render_widget(
                Paragraph::new(t!("tui.too_small").into_owned()).wrap(Wrap { trim: true }),
                area,
            );
            return;
        }
        self.wide = area.width >= WIDE;
        let (activity, open) = self.activity(usize::from(area.width) - 4);
        let activity = Paragraph::new(activity).wrap(WRAP);
        // Borders and tiles only when the window has the rows for them.
        let tall = area.height >= 20;
        let [header, main, activity_area, footer] = Layout::vertical([
            Constraint::Length(if tall { 3 } else { 1 }),
            Constraint::Min(1),
            Constraint::Length(if open {
                activity.line_count(area.width - 4).min(4) as u16 + 2
            } else {
                1
            }),
            Constraint::Length(1),
        ])
        .areas(area);
        self.draw_header(f, header);
        let (mut content, mut finished) = (None, None);
        if self.items.is_empty() {
            draw_empty(f, main);
        } else {
            let (list, preview) = if self.wide {
                let [l, p] = Layout::horizontal([
                    Constraint::Length((main.width / 5 * 2).clamp(38, 60)),
                    Constraint::Min(1),
                ])
                .areas(main);
                (Some(l), Some(p))
            } else if self.focus == Focus::List {
                (Some(main), None)
            } else {
                (None, Some(main))
            };
            if let Some(a) = list {
                finished = self.draw_list(f, a);
            }
            if let Some(a) = preview {
                content = Some(self.draw_preview(f, a));
            }
        }
        if open {
            let block = pane(Line::from(format!(" {} ", t!("tui.activity"))), false);
            f.render_widget(
                activity.block(block.padding(Padding::horizontal(1))),
                activity_area,
            );
        } else {
            f.render_widget(activity, activity_area.inner(Margin::new(2, 0)));
        }
        self.finished = None;
        self.draw_footer(f, footer);
        if let Some(sel) = self.picker {
            draw_picker(f, main, sel);
        }
        if self.help {
            draw_help(f, area);
        }
        if let Some((text, style, _)) = &self.toast {
            draw_toast(f, main, text, *style);
        }
        self.effects(f, elapsed, content, finished);
    }

    /// Starts the effects asked for since the last frame, then plays the running ones. They
    /// only hide cells or use named colors: no 24-bit color, nothing a light theme breaks.
    fn effects(
        &mut self,
        f: &mut Frame,
        mut elapsed: Duration,
        content: Option<Rect>,
        finished: Option<Rect>,
    ) {
        let mut start = Vec::new();
        // Moving fast through the list (a held key) changes the preview without the effect:
        // it would hide the content most of the time.
        let settled = self.swapped.elapsed() >= Duration::from_millis(300);
        if self.swap {
            self.swapped = Instant::now();
        }
        if self.reveal {
            start.push((Fx::Reveal, sweep(300, f.area())));
        } else if let Some(area) = content.filter(|_| self.swap && settled) {
            start.push((Fx::Preview, sweep(160, area)));
        }
        if let Some(row) = finished {
            let flash = fx::paint_fg(Color::Green, 300u32).with_area(row);
            start.push((Fx::Finished, flash));
        }
        (self.reveal, self.swap) = (false, false);
        if motion() && !start.is_empty() {
            // A new effect starts now, not at the previous frame (maybe 250 ms ago).
            elapsed = Duration::ZERO;
            for (key, effect) in start {
                self.fx.add_unique_effect(key, effect);
            }
        }
        let area = f.area();
        self.fx
            .process_effects(elapsed.into(), f.buffer_mut(), area);
    }

    fn draw_header(&self, f: &mut Frame, area: Rect) {
        let count = |s| self.items.iter().filter(|i| i.status == s).count();
        let processing = count(RecordingStatus::Processing);
        let failed = count(RecordingStatus::Failed);
        let mut status = vec![Span::raw(" ")];
        if lock(&self.pull).running {
            status.push(Span::styled(
                format!("{} {}", spinner(), t!("tui.pulling")),
                theme::accent(),
            ));
            status.push(dim(" · "));
        }
        status.push(if processing > 0 {
            Span::styled(
                format!("{} {}", spinner(), t!("tui.processing", count = processing)),
                theme::accent(),
            )
        } else {
            Span::styled(format!("● {}", t!("tui.idle")), theme::ok())
        });
        status.push(dim(" · "));
        status.push(if self.device {
            Span::styled(format!("● {} ", t!("tui.device_linked")), theme::ok())
        } else {
            dim(&format!("○ {} ", t!("tui.device_none")))
        });
        let status = Line::from(status);

        let minutes = self.items.iter().filter_map(|i| i.duration_s).sum::<f64>() as u64 / 60;
        let audio = if minutes >= 60 {
            format!("{} h {:02}", minutes / 60, minutes % 60)
        } else {
            format!("{minutes} min")
        };
        let last_pull = self.last_pull.map_or_else(
            || t!("tui.never").into_owned(),
            |at| {
                let today = local(at, "%F") == local(jiff::Timestamp::now(), "%F");
                local(at, if today { "%H:%M" } else { "%m-%d %H:%M" })
            },
        );
        let hot = |n: usize, style: Style| if n > 0 { style } else { theme::dim() };
        // By importance: the last ones are dropped when the window is too narrow.
        let values = [
            (
                t!("tui.tile_recordings"),
                self.items.len().to_string(),
                theme::bold(),
            ),
            (
                t!("tui.st_processing"),
                processing.to_string(),
                hot(processing, theme::accent()),
            ),
            (
                t!("tui.st_failed"),
                failed.to_string(),
                hot(failed, theme::error()),
            ),
            (
                t!("tui.tile_actions"),
                self.open_actions.to_string(),
                hot(self.open_actions, theme::bold()),
            ),
            (t!("tui.tile_audio"), audio, theme::bold()),
            (t!("tui.tile_last_pull"), last_pull, theme::bold()),
        ];
        let tiles = |max: usize| {
            let mut line = Line::default();
            for (label, value, style) in &values {
                let gap = if line.spans.is_empty() { "" } else { "   " };
                if line.width() + width(gap) + width(label) + 1 + width(value) > max {
                    break;
                }
                line.push_span(dim(&format!("{gap}{label} ")));
                line.push_span(Span::styled(value.clone(), *style));
            }
            line
        };
        if area.height >= 3 {
            let mut brand = vec![Span::raw(" ")];
            brand.extend(theme::gradient("liberarr"));
            brand.push(Span::raw(" "));
            let block = pane(Line::from(brand), false).title(status.right_aligned());
            let inner = block.inner(area).inner(Margin::new(1, 0));
            f.render_widget(block, area);
            f.render_widget(Paragraph::new(tiles(usize::from(inner.width))), inner);
        } else {
            let [left, right] = Layout::horizontal([
                Constraint::Min(1),
                Constraint::Length(status.width() as u16),
            ])
            .areas(area);
            let mut line = Line::from(theme::gradient("▌liberarr"));
            line.push_span("  ");
            // The brand and its gap take 11 columns; 2 more stay free before the status.
            line.extend(tiles(usize::from(left.width).saturating_sub(13)));
            f.render_widget(Paragraph::new(line), left);
            f.render_widget(Paragraph::new(status), right);
        }
    }

    /// Draws the recordings. Returns the row of the recording that just finished, when it is
    /// on screen.
    fn draw_list(&mut self, f: &mut Frame, area: Rect) -> Option<Rect> {
        let focused = self.focus == Focus::List;
        let shown = self.visible().len();
        let count = if shown == self.items.len() {
            shown.to_string()
        } else {
            format!("{shown}/{}", self.items.len())
        };
        let block = pane(
            Line::from(format!(" {} ({count}) ", t!("tui.recordings"))),
            focused,
        );
        let mut rows = block.inner(area);
        f.render_widget(block, area);
        if self.filtering || !self.filter.is_empty() {
            let [bar, rest] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(rows);
            rows = rest;
            let search = Line::from(vec![
                Span::styled(" / ", theme::accent()),
                Span::raw(self.filter.clone()),
                Span::styled(if self.filtering { "▏" } else { "" }, theme::accent()),
            ]);
            f.render_widget(Paragraph::new(search), bar);
        }
        (self.areas.list, self.areas.rows) = (area, rows);
        let visible = self.visible();
        if visible.is_empty() {
            let none = Line::styled(format!(" {}", t!("tui.no_match")), theme::dim());
            f.render_widget(Paragraph::new(none), rows);
            return None;
        }
        // Two columns go to the selection mark.
        let max = usize::from(rows.width).saturating_sub(2);
        let items: Vec<ListItem> = visible.iter().map(|i| ListItem::new(row(i, max))).collect();
        let hidden = visible.len().saturating_sub(usize::from(rows.height));
        let finished = self
            .finished
            .and_then(|id| visible.iter().position(|i| i.id == id));
        f.render_stateful_widget(
            List::new(items)
                .highlight_style(if focused {
                    theme::selected()
                } else {
                    theme::accent()
                })
                .highlight_symbol("▶ "),
            rows,
            &mut self.list,
        );
        scrollbar(f, rows, area, hidden, self.list.offset());
        let line = finished?.checked_sub(self.list.offset())?;
        (line < usize::from(rows.height))
            .then(|| Rect::new(rows.x, rows.y + line as u16, rows.width, 1))
    }

    /// Draws the selected recording: status, pipeline, tabs and the content of the open tab.
    /// Returns the content area.
    fn draw_preview(&mut self, f: &mut Frame, area: Rect) -> Rect {
        let focused = self.focus == Focus::Preview;
        self.areas.preview = area;
        let Some(d) = &self.detail else {
            f.render_widget(pane(Line::default(), focused), area);
            return area;
        };
        let item = self.items.iter().find(|i| i.id == d.id);
        let name = d
            .summary
            .as_ref()
            .map(|s| s.title.clone())
            .or_else(|| item.map(title))
            .unwrap_or_else(|| t!("tui.untitled").into_owned());
        let name = fit(&name, usize::from(area.width).saturating_sub(6));
        let block = pane(Line::from(format!(" {name} ")), focused);
        let inner = block.inner(area).inner(Margin::new(1, 0));
        f.render_widget(block, area);
        let error = latest_by_step(&d.jobs)
            .into_iter()
            .rfind(|j| j.status == JobStatus::Failed)
            .and_then(|j| j.error.as_deref());
        let mut status = Line::default();
        if let Some(i) = item {
            let (mark, style) = status_mark(i);
            let word = match i.status {
                RecordingStatus::Done if i.no_speech => t!("no_speech.marker"),
                RecordingStatus::Done => t!("tui.st_done"),
                RecordingStatus::Processing => t!("tui.st_processing"),
                RecordingStatus::Failed => t!("tui.st_failed"),
            };
            let mut spans = vec![
                Span::styled(format!("{mark} {word}"), style),
                dim(" · "),
                Span::raw(local(i.created_at, "%Y-%m-%d %H:%M")),
            ];
            if let Some(s) = i.duration_s {
                spans.extend([dim(" · "), Span::raw(timestamp(s))]);
            }
            status = Line::from(spans);
        }
        // The pipeline goes next to the status when both fit, under it otherwise. The rule
        // under the tabs is dropped on a short window.
        let steps = stepper(&d.jobs);
        let one_row = status.width() + 3 + steps.width() <= usize::from(inner.width);
        let [meta, steps_area, error_area, tabs, rule, body] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(u16::from(!one_row)),
            Constraint::Length(u16::from(error.is_some())),
            Constraint::Length(1),
            Constraint::Length(u16::from(inner.height >= 12)),
            Constraint::Min(0),
        ])
        .areas(inner);
        f.render_widget(Paragraph::new(status), meta);
        if one_row {
            f.render_widget(Paragraph::new(steps.right_aligned()), meta);
        } else {
            f.render_widget(Paragraph::new(steps), steps_area);
        }
        f.render_widget(
            Paragraph::new(dim(&"─".repeat(usize::from(rule.width)))),
            rule,
        );
        if let Some(e) = error {
            let e = fit(e.lines().next().unwrap_or(""), usize::from(body.width));
            f.render_widget(Paragraph::new(Line::styled(e, theme::error())), error_area);
        }

        let actions = d.summary.as_ref().map_or(&[][..], |s| &s.actions);
        let done = actions.iter().filter(|a| a.done).count();
        let mut actions_title = t!("tui.tab_actions").into_owned();
        if !actions.is_empty() {
            actions_title += &format!(" {done}/{}", actions.len());
        }
        let titles = [
            t!("tui.tab_summary").into_owned(),
            t!("tui.tab_transcript").into_owned(),
            t!("tui.tab_mindmap").into_owned(),
            actions_title,
            t!("tui.tab_jobs").into_owned(),
        ];
        // Each title takes its width and the two columns of padding after it.
        let mut x = tabs.x;
        self.areas.tabs = titles
            .iter()
            .map(|t| {
                let w = width(t) as u16 + 2;
                x += w;
                Rect::new(x - w, tabs.y, w, 1).intersection(tabs)
            })
            .collect();
        f.render_widget(
            Tabs::new(titles)
                .select(self.tab)
                .highlight_style(theme::selected())
                .divider("")
                .padding("", "  "),
            tabs,
        );

        let none = |key: &str| vec![Line::styled(t!(key).into_owned(), theme::dim())];
        let no_summary = if d.transcript.as_ref().is_some_and(|t| !t.has_speech()) {
            "no_speech.detail"
        } else {
            "tui.no_summary"
        };
        // The cursor of the actions shows, and drives the scroll, while the preview has the
        // focus. A new summary may have fewer actions: the cursor stays on one.
        self.action = self.action.min(actions.len().saturating_sub(1));
        let cursor = (focused && TABS[self.tab] == Tab::Actions && !actions.is_empty())
            .then_some(self.action);
        let lines: Vec<Line> = match TABS[self.tab] {
            Tab::Summary => d
                .summary
                .as_ref()
                .map_or_else(|| none(no_summary), |s| markdown(&s.summary_md)),
            Tab::Transcript => match &d.transcript {
                None => none("tui.no_transcript"),
                Some(t) if !t.has_speech() => none("no_speech.detail"),
                Some(t) if t.segments.is_empty() => {
                    t.text.lines().map(|l| Line::raw(l.to_string())).collect()
                }
                Some(t) => t
                    .segments
                    .iter()
                    .map(|s| {
                        let mut spans = vec![dim(&format!("{}  ", timestamp(s.start)))];
                        let who = d
                            .diarization
                            .as_ref()
                            .and_then(|z| speaker_for(s.start, s.end, &z.turns));
                        if let Some(n) = who {
                            let label = speaker_label(n, &d.names, &t!("tui.speaker"));
                            spans.push(Span::styled(format!("{label}: "), theme::accent()));
                        }
                        spans.push(Span::raw(s.text.trim().to_string()));
                        Line::from(spans)
                    })
                    .collect(),
            },
            Tab::Mindmap => d
                .summary
                .as_ref()
                .map_or_else(|| none(no_summary), |s| mindmap(&s.mindmap)),
            Tab::Actions if d.summary.is_none() => none(no_summary),
            Tab::Actions if actions.is_empty() => none("tui.no_actions"),
            Tab::Actions => actions
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    let check = if a.done { "[x] " } else { "[ ] " };
                    let extra: Vec<String> =
                        [a.owner.clone(), a.due.clone(), a.start.map(timestamp)]
                            .into_iter()
                            .flatten()
                            .collect();
                    let style = if cursor == Some(i) {
                        theme::selected()
                    } else if a.done {
                        theme::dim()
                    } else {
                        Style::new()
                    };
                    Line::from(vec![
                        Span::styled(format!("{check}{}", a.action), style),
                        dim(&format!("  {}", extra.join(" · "))),
                    ])
                })
                .collect(),
            Tab::Jobs => d
                .jobs
                .iter()
                .map(|j| {
                    let (mark, style) = match j.status {
                        JobStatus::Done => ("●".to_string(), theme::ok()),
                        JobStatus::Failed => ("▲".to_string(), theme::error()),
                        JobStatus::Running => (spinner().to_string(), theme::accent()),
                        JobStatus::Queued => ("○".to_string(), theme::dim()),
                    };
                    Line::from(vec![
                        Span::styled(format!("{mark} {:<11}", j.step), style),
                        dim(&format!(
                            "{}  ×{}  ",
                            local(j.updated_at, "%Y-%m-%d %H:%M"),
                            j.attempts
                        )),
                        Span::styled(j.error.clone().unwrap_or_default(), theme::error()),
                    ])
                })
                .collect(),
        };

        // Rows once wrapped: of the lines before the cursor, up to it, and of them all.
        let rows = |lines: &[Line<'static>]| {
            Paragraph::new(lines.to_vec())
                .wrap(WRAP)
                .line_count(body.width)
        };
        let around = cursor.map(|at| (rows(&lines[..at]), rows(&lines[..=at])));
        let content = Paragraph::new(lines).wrap(WRAP);
        let height = usize::from(body.height);
        let hidden = content.line_count(body.width).saturating_sub(height);
        let mut scroll = usize::from(self.scroll).min(hidden);
        if let Some((top, bottom)) = around {
            scroll = scroll.max(bottom.saturating_sub(height)).min(top);
        }
        (self.scroll, self.page) = (scroll as u16, body.height);
        f.render_widget(content.scroll((self.scroll, 0)), body);
        scrollbar(f, body, area, hidden, scroll);
        body
    }

    /// The device pull and the worker, as lines, and whether the pane is open: something
    /// runs, or a pull ended a moment ago. Closed, it is one line.
    fn activity(&self, max: usize) -> (Vec<Line<'static>>, bool) {
        let pull = lock(&self.pull);
        let mut lines = Vec::new();
        let mut recent = false;
        match &pull.phase {
            Phase::Idle => {}
            Phase::Searching => lines.push(Line::styled(
                format!("{} {}", spinner(), t!("pull.searching")),
                theme::accent(),
            )),
            Phase::Downloading(tr) => lines.push(transfer(tr, pull.downloaded + 1, max)),
            Phase::Finished(outcome, failed, at) => {
                recent = at.elapsed() < OUTCOME;
                let style = if *failed { theme::warn() } else { theme::ok() };
                lines.extend(
                    outcome
                        .iter()
                        .map(|l| Line::styled(l.trim().to_string(), style)),
                );
            }
        }
        let processing: Vec<String> = self
            .items
            .iter()
            .filter(|i| i.status == RecordingStatus::Processing)
            .map(title)
            .collect();
        if !processing.is_empty() {
            lines.push(Line::from(vec![
                Span::styled(
                    format!(
                        "{} {}",
                        spinner(),
                        t!("tui.processing", count = processing.len())
                    ),
                    theme::accent(),
                ),
                dim(" · "),
                Span::raw(processing.join(" · ")),
            ]));
        }
        if pull.running || !processing.is_empty() || recent {
            return (lines, true);
        }
        // Closed: the title, then the outcome of the last pull or how to start one.
        let mut line = Line::from(dim(&format!("{}  ", t!("tui.activity"))));
        match lines.into_iter().next() {
            Some(outcome) => line.extend(outcome),
            None => line.push_span(dim(&t!("tui.activity_idle"))),
        }
        // One line: the paragraph wraps, so the text is cut here.
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        if width(&text) > max {
            line = Line::from(dim(&fit(&text, max)));
        }
        (vec![line], false)
    }

    fn draw_footer(&self, f: &mut Frame, area: Rect) {
        let keys = if self.filtering {
            t!("tui.hints_filter")
        } else if self.focus == Focus::Preview {
            t!("tui.hints_detail")
        } else {
            t!("tui.hints_list")
        };
        // Help and quit stay visible when the window is too narrow for every hint. Not while
        // typing a search: every character goes to it.
        let more = if self.filtering {
            Line::default()
        } else {
            hints(&t!("tui.hints_more"), usize::from(area.width))
        };
        let [left, right] =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(more.width() as u16)])
                .areas(area);
        f.render_widget(Paragraph::new(hints(&keys, usize::from(left.width))), left);
        f.render_widget(Paragraph::new(more), right);
    }
}

/// No recording yet: the logo and how to get one.
fn draw_empty(f: &mut Frame, area: Rect) {
    let block = pane(Line::from(format!(" {} (0) ", t!("tui.recordings"))), true);
    let inner = block.inner(area).inner(Margin::new(2, 0));
    f.render_widget(block, area);
    let mut lines: Vec<Line> = LOGO
        .iter()
        .map(|l| Line::from(theme::gradient(l)))
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::raw(t!("tui.empty").into_owned()));
    let [c] = Layout::vertical([Constraint::Length(7)])
        .flex(Flex::Center)
        .areas(inner);
    f.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        c,
    );
}

fn draw_picker(f: &mut Frame, area: Rect, sel: usize) {
    let r = popup(f, area, 34, STEPS.len() as u16 + 2);
    let lines: Vec<Line> = STEPS
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let style = if i == sel {
                theme::selected()
            } else {
                Style::new()
            };
            Line::styled(format!(" {s} "), style)
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).block(pane(Line::from(t!("tui.rerun_title").into_owned()), true)),
        r,
    );
}

/// Every key, over the dashboard. Each row of the catalog is `keys|what they do`.
fn draw_help(f: &mut Frame, area: Rect) {
    let rows: Vec<(String, String)> = [
        "tui.help_move",
        "tui.help_pane",
        "tui.help_tab",
        "tui.help_page",
        "tui.help_tick",
        "tui.help_search",
        "tui.help_pull",
        "tui.help_rerun",
        "tui.help_settings",
        "tui.help_mouse",
        "tui.help_back",
        "tui.help_quit",
    ]
    .iter()
    .map(|key| {
        let row = t!(*key);
        let (keys, what) = row.split_once('|').unwrap_or((row.as_ref(), ""));
        (keys.to_string(), what.to_string())
    })
    .collect();
    let keys = rows.iter().map(|(k, _)| width(k)).max().unwrap_or(0);
    let what = rows.iter().map(|(_, w)| width(w)).max().unwrap_or(0);
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, w)| {
            let pad = " ".repeat(keys - width(k) + 2);
            Line::from(vec![
                Span::styled(format!(" {k}{pad}"), theme::bold()),
                Span::raw(w.clone()),
            ])
        })
        .collect();
    let r = popup(f, area, (keys + what + 6) as u16, rows.len() as u16 + 2);
    let title = Line::from(format!(" {} ", t!("tui.help_title")));
    f.render_widget(Paragraph::new(lines).block(pane(title, true)), r);
}

/// A notice in a box at the bottom right of the panes, for a few seconds.
fn draw_toast(f: &mut Frame, area: Rect, text: &str, style: Style) {
    let w = (width(text) as u16)
        .saturating_add(4)
        .min(area.width.saturating_sub(2));
    let notice = Paragraph::new(Line::styled(text.to_string(), style))
        .wrap(Wrap { trim: true })
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(style)
                .padding(Padding::horizontal(1)),
        );
    let h = (notice.line_count(w) as u16).min(area.height);
    // Inside the border of the pane it covers.
    let r = Rect::new(
        area.right().saturating_sub(w + 2),
        area.bottom().saturating_sub(h + 1).max(area.y),
        w,
        h,
    );
    f.render_widget(Clear, r);
    f.render_widget(notice, r);
}
