//! Content language: the text Liberarr generates into notes and asks the model to write. Pure data,
//! no i18n library: French is the original, English its faithful equivalent.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    En,
    Fr,
}

/// Headings and fixed lines of the Obsidian note block.
pub struct NoteLabels {
    pub summary: &'static str,
    pub mindmap: &'static str,
    pub actions: &'static str,
    pub no_actions: &'static str,
    pub transcript: &'static str,
    /// Default speaker label: "Speaker 1".
    pub speaker: &'static str,
}

impl Language {
    /// Accepts `en`, `fr` and locale tags such as `fr_FR.UTF-8` or `fr-FR`, in any case.
    pub fn parse(tag: &str) -> Option<Language> {
        let primary = tag.split(['_', '-', '.', '@']).next()?;
        match primary.to_ascii_lowercase().as_str() {
            "en" => Some(Language::En),
            "fr" => Some(Language::Fr),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Language::En => "en",
            Language::Fr => "fr",
        }
    }

    pub fn note_labels(self) -> NoteLabels {
        match self {
            Language::Fr => NoteLabels {
                summary: "Résumé",
                mindmap: "Carte mentale",
                actions: "Actions",
                no_actions: "Aucune action.",
                transcript: "Transcription",
                speaker: "Locuteur",
            },
            Language::En => NoteLabels {
                summary: "Summary",
                mindmap: "Mind map",
                actions: "Actions",
                no_actions: "No actions.",
                transcript: "Transcript",
                speaker: "Speaker",
            },
        }
    }

    /// The summary system prompt: same JSON shape and rules in both languages.
    pub fn summary_prompt(self) -> &'static str {
        match self {
            Language::Fr => {
                r#"Tu es l'assistant de compte rendu de réunion. On te donne la transcription d'un enregistrement (français, parfois mêlé d'anglais), horodatée ligne par ligne quand c'est possible. Réponds UNIQUEMENT avec un objet JSON de cette forme :
{
  "title": "titre court et explicite",
  "summary_md": "compte rendu en Markdown avec les sections ## Contexte, ## Points clés, ## Décisions",
  "mindmap": "carte mentale Mermaid commençant par la ligne 'mindmap', racine = sujet principal",
  "actions": [{"action": "...", "owner": "personne nommée dans la transcription ou null",
               "due": "échéance seulement si elle est dite, sinon null",
               "start": secondes du segment où l'action est énoncée, ou null}]
}
N'invente rien qui ne soit pas dans la transcription. Rédige en français."#
            }
            Language::En => {
                r#"You are a meeting-minutes assistant. You are given the transcript of a recording (possibly French, sometimes mixed with English), timestamped line by line when possible. Reply ONLY with a JSON object of this shape:
{
  "title": "short, explicit title",
  "summary_md": "minutes in Markdown with the sections ## Context, ## Key points, ## Decisions",
  "mindmap": "Mermaid mind map starting with the line 'mindmap', root = main subject",
  "actions": [{"action": "...", "owner": "person named in the transcript or null",
               "due": "deadline only if it is stated, otherwise null",
               "start": seconds of the segment where the action is stated, or null}]
}
Invent nothing that is not in the transcript. Write the summary, its section headings and the title in English, whatever the language of the recording."#
            }
        }
    }

    /// Sent back to the model after an invalid reply.
    pub fn retry_message(self, problem: &str) -> String {
        match self {
            Language::Fr => format!(
                "Réponse invalide : {problem}. Renvoie uniquement le JSON demandé, corrigé."
            ),
            Language::En => {
                format!("Invalid reply: {problem}. Return only the requested JSON, corrected.")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_locale_tags() {
        for (tag, want) in [
            ("en", Some(Language::En)),
            ("FR", Some(Language::Fr)),
            ("fr_FR.UTF-8", Some(Language::Fr)),
            ("fr-FR", Some(Language::Fr)),
            ("en_US.UTF-8", Some(Language::En)),
            ("de_DE", None),
            ("C", None),
            ("", None),
        ] {
            assert_eq!(Language::parse(tag), want, "{tag}");
        }
    }
}
