//! Search inside the corrected transcripts: one folding rule, the query rule, the excerpt and the scan.
use crate::Result;
use crate::recordings::RecordingItem;
use crate::transcription::TranscriptRepo;

pub const MIN_QUERY_CHARS: usize = 2;
pub const MAX_QUERY_CHARS: usize = 200;
/// Characters kept on each side of a match.
pub const EXCERPT_CHARS: usize = 60;

fn is_combining(c: char) -> bool {
    matches!(c, '\u{300}'..='\u{36f}')
}

/// Pushes the folded form of one character (lowercased alone, then the table; combining accents push nothing).
// shortcut: only the accented Latin letters of French and nearby languages are folded; other scripts compare as they are. Upgrade to Unicode normalization if users search other languages.
fn fold_char(c: char, out: &mut String) {
    for c in c.to_lowercase() {
        match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => out.push('a'),
            'ç' => out.push('c'),
            'è' | 'é' | 'ê' | 'ë' => out.push('e'),
            'ì' | 'í' | 'î' | 'ï' => out.push('i'),
            'ñ' => out.push('n'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => out.push('o'),
            'ù' | 'ú' | 'û' | 'ü' => out.push('u'),
            'ý' | 'ÿ' => out.push('y'),
            'œ' => out.push_str("oe"),
            'æ' => out.push_str("ae"),
            '’' | '‘' => out.push('\''),
            // Combining accents: "é" typed as "e" + U+0301 (macOS file names) folds too.
            c if is_combining(c) => {}
            c => out.push(c),
        }
    }
}

/// Lowercase, accents removed: "Réunion" and "reunion" compare equal.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        fold_char(c, &mut out);
    }
    out
}

/// Trimmed, `MIN_QUERY_CHARS`..=`MAX_QUERY_CHARS` characters, no control character.
pub fn query_ok(q: &str) -> bool {
    let q = q.trim();
    (MIN_QUERY_CHARS..=MAX_QUERY_CHARS).contains(&q.chars().count())
        && !q.chars().any(char::is_control)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Excerpt {
    pub before: String,
    pub hit: String,
    pub after: String,
}

/// The first match of the already folded, non-empty query in `text`, in the original spelling.
pub fn excerpt(text: &str, folded_query: &str) -> Option<Excerpt> {
    if folded_query.is_empty() {
        return None;
    }
    let text = text.trim();
    // origin[i] = byte offset in `text` of the character that produced byte i of `folded`.
    let mut folded = String::with_capacity(text.len());
    let mut origin = Vec::with_capacity(text.len());
    for (at, c) in text.char_indices() {
        let from = folded.len();
        fold_char(c, &mut folded);
        origin.resize(origin.len() + folded.len() - from, at);
    }
    let found = folded.find(folded_query)?;
    let start = origin[found];
    let last = origin[found + folded_query.len() - 1];
    let mut end = last + text[last..].chars().next()?.len_utf8();
    end += text[end..]
        .chars()
        .take_while(|c| is_combining(*c))
        .map(char::len_utf8)
        .sum::<usize>();

    let head = &text[..start];
    let (before, cut) = match head.char_indices().rev().nth(EXCERPT_CHARS - 1) {
        Some((i, _)) if i > 0 => (head[i..].trim_start_matches(is_combining), true),
        _ => (head, false),
    };
    let tail = &text[end..];
    let (after, cut_after) = match tail.char_indices().nth(EXCERPT_CHARS) {
        Some((i, _)) => (&tail[..i], true),
        None => (tail, false),
    };
    Some(Excerpt {
        before: if cut {
            format!("…{before}")
        } else {
            before.into()
        },
        hit: text[start..end].into(),
        after: if cut_after {
            format!("{after}…")
        } else {
            after.into()
        },
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptHit {
    pub recording_id: i64,
    pub start: Option<f64>,
    pub excerpt: Excerpt,
}

/// One hit per unit (a segment, or the whole text without timestamps) of each recording's corrected transcript,
/// in `items` order then time order; stops after `max` hits; nothing when the query folds to "".
// shortcut: a full scan of every corrected transcript per query (~90 ms for 300 one-hour recordings, ~450 ms for 1,500 hours); move to an FTS5 index kept in step with the overlay when a search takes over ~300 ms.
pub async fn search_transcripts(
    transcripts: &impl TranscriptRepo,
    items: &[RecordingItem],
    query: &str,
    max: usize,
) -> Result<Vec<TranscriptHit>> {
    let q = fold(query.trim());
    let mut hits = Vec::new();
    if q.is_empty() {
        return Ok(hits);
    }
    for item in items {
        if hits.len() >= max {
            break;
        }
        let Some(t) = transcripts.get(item.id).await? else {
            continue;
        };
        let units: Vec<(Option<f64>, &str)> = if t.has_timestamps {
            t.segments
                .iter()
                .map(|s| (Some(s.start), s.text.as_str()))
                .collect()
        } else {
            vec![(None, t.text.as_str())]
        };
        for (start, text) in units {
            if hits.len() >= max {
                break;
            }
            // The cheap test first: the offset map of `excerpt` is built only for a unit that matches.
            if !fold(text.trim()).contains(&q) {
                continue;
            }
            if let Some(excerpt) = excerpt(text, &q) {
                hits.push(TranscriptHit {
                    recording_id: item.id,
                    start,
                    excerpt,
                });
            }
        }
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::fakes::{MemTranscripts, block_on};
    use crate::recordings::{RecordingStatus, Source};
    use crate::transcription::{Segment, Transcript};

    #[test]
    fn fold_ignores_case_accents_and_curly_apostrophes() {
        assert_eq!(fold("Réunion"), "reunion");
        assert_eq!(fold("Re\u{301}union"), "reunion");
        assert_eq!(fold("Cœur"), "coeur");
        assert_eq!(fold("ÆON"), "aeon");
        assert_eq!(fold("D’accord"), "d'accord");
        assert_eq!(fold("‘ok’"), "'ok'");
        assert_eq!(fold("Sánchez"), "sanchez");
        assert_eq!(fold("João"), "joao");
        assert_eq!(fold("Ångström"), "angstrom");
        assert_eq!(fold("Ýves"), "yves");
    }

    #[test]
    fn query_rule() {
        assert!(query_ok(" ré "));
        assert!(!query_ok(" œ "));
        assert!(query_ok(&"é".repeat(MAX_QUERY_CHARS)));
        assert!(!query_ok(&"é".repeat(MAX_QUERY_CHARS + 1)));
        assert!(!query_ok("a\u{1}b"));
        assert!(!query_ok("a\nb"));
    }

    #[test]
    fn excerpt_keeps_the_original_text_around_the_match() {
        let e = excerpt("  On parle de la Réunion d'équipe \n", &fold("reunion")).unwrap();
        assert_eq!(
            e,
            Excerpt {
                before: "On parle de la ".into(),
                hit: "Réunion".into(),
                after: " d'équipe".into()
            }
        );
        assert_eq!(
            excerpt("on code en C++ ici", &fold("C++")).unwrap().hit,
            "C++"
        );
        assert_eq!(
            excerpt("100% (\"vrai\") _x_", &fold("% (\"")).unwrap().hit,
            "% (\""
        );
        assert!(excerpt("Bonjour", &fold("réunion")).is_none());
        assert!(excerpt("Bonjour", "").is_none());
    }

    #[test]
    fn excerpt_cuts_on_character_boundaries() {
        let text = format!("{}Cœur{}", "é".repeat(200), "ü".repeat(200));
        let e = excerpt(&text, &fold("coeur")).unwrap();
        assert_eq!(e.before, format!("…{}", "é".repeat(EXCERPT_CHARS)));
        assert_eq!(e.hit, "Cœur");
        assert_eq!(e.after, format!("{}…", "ü".repeat(EXCERPT_CHARS)));
        assert_eq!(excerpt("Cœur", "co").unwrap().hit, "Cœ");
        assert_eq!(excerpt("Cœur", "eur").unwrap().hit, "œur");
        assert_eq!(excerpt("Re\u{301}union", "re").unwrap().hit, "Re\u{301}");
        // The last 60 characters before the match start with U+0301: the excerpt skips it.
        let text = format!(
            "{}e\u{301}{}x Cœur",
            "a".repeat(10),
            "b".repeat(EXCERPT_CHARS - 3)
        );
        assert_eq!(
            excerpt(&text, "coeur").unwrap().before,
            format!("…{}x ", "b".repeat(EXCERPT_CHARS - 3))
        );
    }

    fn item(id: i64) -> RecordingItem {
        RecordingItem {
            id,
            title: None,
            source: Source::File,
            duration_s: None,
            created_at: jiff::Timestamp::UNIX_EPOCH,
            status: RecordingStatus::Done,
            failed_step: None,
            no_speech: false,
        }
    }

    fn transcript(segments: &[(f64, &str)], text: &str) -> Transcript {
        Transcript {
            engine: "e".into(),
            language: None,
            text: text.into(),
            segments: segments
                .iter()
                .map(|(start, text)| Segment {
                    start: *start,
                    end: *start + 1.0,
                    text: (*text).into(),
                })
                .collect(),
            has_timestamps: !segments.is_empty(),
        }
    }

    #[test]
    fn search_lists_one_hit_per_unit_in_order_and_stops_at_max() {
        // items: ids 2, 1, 3 (newest first, as list_items returns them).
        let items = [item(2), item(1), item(3)];
        let repo = MemTranscripts::default();
        repo.map.borrow_mut().insert(
            1,
            transcript(
                &[
                    (0.0, "Réunion, réunion !"),
                    (5.0, "Bonjour."),
                    (10.0, "La réunion."),
                ],
                "Réunion, réunion ! Bonjour. La réunion.",
            ),
        );
        repo.map
            .borrow_mut()
            .insert(2, transcript(&[], "Une réunion."));
        let hits = block_on(search_transcripts(&repo, &items, "  REUNION ", 10)).unwrap();
        let at: Vec<_> = hits.iter().map(|h| (h.recording_id, h.start)).collect();
        assert_eq!(at, [(2, None), (1, Some(0.0)), (1, Some(10.0))]);
        assert_eq!(
            block_on(search_transcripts(&repo, &items, "reunion", 2))
                .unwrap()
                .len(),
            2
        );
        assert!(
            block_on(search_transcripts(&repo, &items, "\u{301}\u{301}", 10))
                .unwrap()
                .is_empty()
        );
    }
}
