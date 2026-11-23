//! Summaries context: LLM output validation, the summarize use case and action toggling.
use crate::lang::Language;
use crate::recordings::{RecordingItem, RecordingRepo};
use crate::transcription::{Transcript, TranscriptRepo, timestamp};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("invalid summary: {0}")]
pub struct InvalidSummary(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub action: String,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
    /// Seconds into the recording where the action is stated.
    #[serde(default)]
    pub start: Option<f64>,
    #[serde(default)]
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeetingSummary {
    pub title: String,
    pub summary_md: String,
    pub mindmap: String,
    #[serde(default)]
    pub actions: Vec<Action>,
}

/// LLM output is untrusted: it must be the exact JSON shape with a Mermaid mindmap.
pub fn parse_summary(text: &str) -> Result<MeetingSummary, InvalidSummary> {
    // Claude models (no forced JSON mode) often wrap the reply in a ```json fence.
    let text = text.trim();
    let text = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .and_then(|t| t.strip_suffix("```"))
        .unwrap_or(text);
    // serde messages can quote the model's text (summary content), so keep only category and position.
    let mut s: MeetingSummary = serde_json::from_str(text).map_err(|e| {
        InvalidSummary(format!(
            "{:?} error at line {} column {}",
            e.classify(),
            e.line(),
            e.column()
        ))
    })?;
    if !s.mindmap.trim_start().starts_with("mindmap") {
        return Err(InvalidSummary(
            "mindmap must be a Mermaid diagram starting with 'mindmap'".into(),
        ));
    }
    s.mindmap = s.mindmap.trim().to_string();
    Ok(s)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }
}

/// System prompt plus the transcript: `[mm:ss] text` lines, or the plain text without timestamps.
pub fn summary_messages(t: &Transcript, lang: Language) -> Vec<ChatMessage> {
    let body = if t.has_timestamps {
        t.segments
            .iter()
            .map(|s| format!("[{}] {}", timestamp(s.start), s.text.trim()))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        t.text.clone()
    };
    vec![
        ChatMessage::new(Role::System, lang.summary_prompt()),
        ChatMessage::new(Role::User, body),
    ]
}

pub trait Llm {
    /// Returns the raw JSON text of the model's reply.
    async fn complete_json(&self, messages: &[ChatMessage]) -> Result<String>;
}

/// The owner's title and summary, both stored whole.
#[derive(Debug, Clone, PartialEq)]
pub struct SummaryText {
    pub title: String,
    pub summary_md: String,
}

pub trait SummaryEditRepo {
    /// The generated summary, untouched, and the owner's title and summary when edited.
    async fn get_with_edit(
        &self,
        recording_id: i64,
    ) -> Result<Option<(MeetingSummary, Option<SummaryText>)>>;
    /// One transaction: `NotFound("summary")` without a summary; `Conflict("summary")` when the stored generated title or
    /// summary_md differ from `generated`, or the effective pair (the stored edit, else the generated one) differs
    /// from `current`, what the page displayed; `edit == generated` deletes the overlay, else it is upserted.
    async fn set_edit(
        &self,
        recording_id: i64,
        generated: &SummaryText,
        current: &SummaryText,
        edit: &SummaryText,
    ) -> Result<()>;
}

pub trait SummaryRepo {
    /// Drops the owner's title and summary when the new title or summary_md differ.
    async fn save(&self, recording_id: i64, summary: &MeetingSummary) -> Result<()>;
    /// `title` and `summary_md` are the owner's when edited.
    async fn get(&self, recording_id: i64) -> Result<Option<MeetingSummary>>;
    /// Sets `done` in one atomic write, only while the action at `index` still reads `action`.
    /// Returns whether it was set.
    async fn set_action_done(
        &self,
        recording_id: i64,
        index: usize,
        action: &str,
        done: bool,
    ) -> Result<bool>;
    /// Number of actions not ticked yet, over all recordings.
    async fn open_actions(&self) -> Result<u32>;
}

pub struct Summarize<'a, T, L, S> {
    pub transcripts: &'a T,
    pub llm: &'a L,
    pub summaries: &'a S,
    pub lang: Language,
}

impl<T: TranscriptRepo, L: Llm, S: SummaryRepo> Summarize<'_, T, L, S> {
    /// One retry with the validation error appended, then `InvalidSummary`. Nothing is saved on failure.
    /// `None` without calling the LLM when the transcript has no speech.
    pub async fn run(&self, recording_id: i64) -> Result<Option<MeetingSummary>> {
        let t = self
            .transcripts
            .get(recording_id)
            .await?
            .ok_or_else(|| Error::NotFound("transcript".into()))?;
        if !t.has_speech() {
            return Ok(None);
        }
        let mut messages = summary_messages(&t, self.lang);
        for attempt in 0..2 {
            let reply = self.llm.complete_json(&messages).await?;
            match parse_summary(&reply) {
                Ok(mut s) => {
                    // A rerun keeps the owner's ticks on the actions whose text is unchanged.
                    if let Some(prev) = self.summaries.get(recording_id).await? {
                        for a in &mut s.actions {
                            a.done = prev.actions.iter().any(|p| p.done && p.action == a.action);
                        }
                    }
                    self.summaries.save(recording_id, &s).await?;
                    return Ok(Some(s));
                }
                Err(e) if attempt == 1 => return Err(e.into()),
                Err(e) => {
                    messages.push(ChatMessage::new(Role::Assistant, reply));
                    messages.push(ChatMessage::new(Role::User, self.lang.retry_message(&e.0)));
                }
            }
        }
        unreachable!("the second attempt always returns")
    }
}

pub struct ToggleAction<'a, S> {
    pub summaries: &'a S,
}

impl<S: SummaryRepo> ToggleAction<'_, S> {
    /// `action` is the text the client saw at `index`: when a rerun has replaced the actions since,
    /// the toggle fails with `Error::Conflict` instead of ticking another action.
    pub async fn run(
        &self,
        recording_id: i64,
        index: usize,
        action: &str,
        done: bool,
    ) -> Result<MeetingSummary> {
        let set = self
            .summaries
            .set_action_done(recording_id, index, action, done)
            .await?;
        let s = self
            .summaries
            .get(recording_id)
            .await?
            .ok_or_else(|| Error::NotFound("summary".into()))?;
        match (set, s.actions.get(index)) {
            (true, _) => Ok(s),
            (false, None) => Err(Error::NotFound("action".into())),
            (false, Some(_)) => Err(Error::Conflict("action".into())),
        }
    }
}

/// One recording and its summary's actions as stored: an action's position is the index `ToggleAction` takes.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordingActions {
    pub item: RecordingItem,
    pub actions: Vec<Action>,
}

/// Every recording whose summary has at least one action, in `list_items` order (newest first).
/// `item.title` is the title the recordings list shows (the owner's when edited).
pub async fn actions_by_recording(
    recordings: &impl RecordingRepo,
    summaries: &impl SummaryRepo,
) -> Result<Vec<RecordingActions>> {
    let mut found = Vec::new();
    // shortcut: one summary query per recording, all read before any date filter of a caller;
    // fine for one person's recordings, add a dedicated query if a home ever holds thousands.
    for item in recordings.list_items().await? {
        let Some(s) = summaries.get(item.id).await? else {
            continue;
        };
        if !s.actions.is_empty() {
            found.push(RecordingActions {
                item,
                actions: s.actions,
            });
        }
    }
    Ok(found)
}

/// The actions not ticked yet, each with its position in the whole list (the toggle's index).
pub fn open_with_index(actions: &[Action]) -> impl Iterator<Item = (usize, &Action)> {
    actions.iter().enumerate().filter(|(_, a)| !a.done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use crate::fakes::{FakeLlm, MemSummaries, MemTranscripts, block_on};

    #[test]
    fn open_with_index_keeps_the_toggle_index() {
        let a = |text: &str, done| Action {
            action: text.into(),
            owner: None,
            due: None,
            start: None,
            done,
        };
        let actions = [a("x", true), a("y", false), a("z", true), a("w", false)];
        let open: Vec<_> = open_with_index(&actions)
            .map(|(i, a)| (i, a.action.as_str()))
            .collect();
        assert_eq!(open, [(1, "y"), (3, "w")]);
    }

    #[test]
    fn parse_summary_rejects_non_mindmap() {
        let json = r#"{"title":"t","summary_md":"s","mindmap":"graph TD\n A-->B","actions":[]}"#;
        assert!(parse_summary(json).is_err());
        let ok = r#"{"title":"t","summary_md":"s","mindmap":"mindmap\n  root","actions":[{"action":"x"}]}"#;
        assert_eq!(parse_summary(ok).unwrap().actions[0].owner, None);
    }

    #[test]
    fn parse_summary_strips_code_fence() {
        let json = r#"{"title":"t","summary_md":"s","mindmap":"mindmap\n  root","actions":[]}"#;
        let expected = parse_summary(json).unwrap();
        for fenced in [
            format!("```json\n{json}\n```"),
            format!("  ```\n{json}\n```\n"),
        ] {
            assert_eq!(parse_summary(&fenced).unwrap(), expected);
        }
        let err = parse_summary("```json\nsecret garbage\n```").unwrap_err();
        assert!(!err.0.contains("secret"));
    }

    #[test]
    fn summarize_retries_once_then_fails() {
        let bad_diagram =
            r#"{"title":"t","summary_md":"s","mindmap":"graph TD\n A-->B","actions":[]}"#;
        let llm = FakeLlm::new(&["not json", bad_diagram]);
        let transcripts = MemTranscripts::default();
        transcripts.map.borrow_mut().insert(
            1,
            Transcript {
                engine: "e".into(),
                language: None,
                text: "salut".into(),
                segments: vec![],
                has_timestamps: false,
            },
        );
        let summaries = MemSummaries::default();
        let uc = Summarize {
            transcripts: &transcripts,
            llm: &llm,
            summaries: &summaries,
            lang: Language::Fr,
        };

        let err = block_on(uc.run(1)).unwrap_err();

        assert!(matches!(err, Error::InvalidSummary(_)));
        assert_eq!(llm.calls.get(), 2);
        assert!(summaries.0.borrow().is_empty());
        assert_eq!(transcripts.saves.get(), 0);
    }

    #[test]
    fn summarize_rerun_keeps_ticks_on_unchanged_actions() {
        let rerun = r#"{"title":"t","summary_md":"s","mindmap":"mindmap\n  root",
            "actions":[{"action":"new one"},{"action":"send deck"}]}"#;
        let llm = FakeLlm::new(&[rerun]);
        let transcripts = MemTranscripts::default();
        transcripts.map.borrow_mut().insert(
            1,
            Transcript {
                engine: "e".into(),
                language: None,
                text: "salut".into(),
                segments: vec![],
                has_timestamps: false,
            },
        );
        let summaries = MemSummaries::default();
        let prev = parse_summary(
            r#"{"title":"t","summary_md":"s","mindmap":"mindmap\n  root",
            "actions":[{"action":"send deck","done":true},{"action":"gone","done":true}]}"#,
        )
        .unwrap();
        summaries.0.borrow_mut().insert(1, prev);
        let uc = Summarize {
            transcripts: &transcripts,
            llm: &llm,
            summaries: &summaries,
            lang: Language::Fr,
        };

        block_on(uc.run(1)).unwrap();

        let saved = summaries.0.borrow()[&1].clone();
        let done: Vec<_> = saved.actions.iter().map(|a| a.done).collect();
        assert_eq!(done, [false, true]);
    }
}
