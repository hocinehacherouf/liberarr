//! Development only: fills an empty directory with a Liberarr home of fictional recordings, for
//! screenshots, demos and manual tests. No device, no account and no real key are needed, and
//! the providers point at a closed local port, so nothing leaves the machine.
//!
//! ```sh
//! cargo run -p liberarr --example demo_home -- /tmp/liberarr-demo
//! LIBERARR_HOME=/tmp/liberarr-demo cargo run -p liberarr -- tui   # in another terminal
//! ```
//!
//! Every name, company and sentence below is invented.
use liberarr_core::device::ports::{DeviceLedger, KeyStore, UnlockMaterial};
use liberarr_core::device::pull::serial_hash;
use liberarr_core::jobs::{JobQueue, STEPS, next_step};
use liberarr_core::recordings::{NewRecording, RecordingRepo, Source};
use liberarr_core::summaries::{Action, MeetingSummary, SummaryRepo};
use liberarr_core::transcription::{Segment, Transcript, TranscriptRepo};
use liberarr_infra::keystore::FileKeyStore;
use liberarr_infra::sqlite::Db;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

/// Not a real device: it is never in range, so a pull from this home ends with "no device".
const SERIAL: &str = "DEMO-0000";

enum State {
    Done,
    /// Transcribed, the summary still running.
    Processing,
    /// The summary failed with this error.
    Failed(&'static str),
    NoSpeech,
}

struct Demo {
    title: &'static str,
    language: &'static str,
    minutes_ago: i64,
    seconds: f64,
    source: Source,
    state: State,
    summary: &'static str,
    mindmap: &'static str,
    /// `(action, owner, due, done)`; an empty owner or due date is none.
    actions: &'static [(&'static str, &'static str, &'static str, bool)],
    /// What is said, spread evenly over the recording.
    speech: &'static [&'static str],
}

const DEMOS: &[Demo] = &[
    Demo {
        title: "Weekly product sync",
        language: "en",
        minutes_ago: 35,
        seconds: 1872.0,
        source: Source::Device,
        state: State::Done,
        summary: "# Decisions
- **Ship the offline mode in 1.4**: the sync conflicts are fixed and the beta feedback is good.
- Onboarding stays at three screens; the tour moves to the help menu.
- The pricing page test runs two more weeks before any change.

# Discussion
- Support tickets dropped by a third since the search rewrite.
- **Risk**: the Android release depends on the store review, which took nine days last time.
- Priya showed the new export flow: exports to CSV are twice as fast.

# Next steps
- Freeze the 1.4 scope on Thursday.
- Share the beta numbers with the whole team.",
        mindmap: "mindmap
  root((Weekly product sync))
    Release 1.4
      Offline mode
      Scope freeze on Thursday
    Onboarding
      Three screens
      Tour in the help menu
    Pricing test
      Two more weeks
    Risks
      Android store review",
        actions: &[
            ("Freeze the 1.4 scope", "Lina", "Thursday", true),
            ("Send the beta numbers to the team", "Tomás", "Friday", true),
            (
                "Submit the Android build for review",
                "Priya",
                "Monday",
                false,
            ),
            ("Move the product tour to the help menu", "Jonas", "", false),
            (
                "Write the release notes for 1.4",
                "Lina",
                "next week",
                false,
            ),
        ],
        speech: &[
            "Okay, let's start. Three topics today: the 1.4 release, onboarding and the pricing test.",
            "On offline mode, the last sync conflicts are fixed. Beta users have had it for two weeks.",
            "Any blocker left? Not on our side. The Android review is the unknown.",
            "Last time the store took nine days, so we should submit on Monday at the latest.",
            "For onboarding, the data says people skip the tour. I would move it to the help menu.",
            "Agreed. Three screens, then straight into the app.",
            "Pricing page: the test is not conclusive yet. Two more weeks.",
            "Support tickets are down by a third since the search rewrite, by the way.",
            "Good. Lina freezes the scope on Thursday, Tomás shares the beta numbers.",
            "Thanks everyone.",
        ],
    },
    Demo {
        title: "Point budget T1",
        language: "fr",
        minutes_ago: 95,
        seconds: 2825.0,
        source: Source::Device,
        state: State::Processing,
        summary: "",
        mindmap: "",
        actions: &[],
        speech: &[
            "Bonjour à tous, on commence par le budget du premier trimestre.",
            "Côté marketing, on est à quatre-vingts pour cent du forecast, c'est plutôt bien.",
            "Le poste cloud a augmenté de douze pour cent, il faut regarder le sizing des machines.",
            "On peut renégocier le contrat en mars, je prépare un benchmark.",
            "Pour les recrutements, deux postes sont gelés jusqu'à la prochaine review.",
            "D'accord. On refait un point dans deux semaines avec les chiffres à jour.",
        ],
    },
    Demo {
        title: "Entretien client : Atelier Morel",
        language: "fr",
        minutes_ago: 5 * 60,
        seconds: 1360.0,
        source: Source::Device,
        state: State::Done,
        summary: "# Contexte
- L'**Atelier Morel** veut remplacer son outil de devis avant l'été.
- Douze personnes l'utiliseraient, dont quatre en mobilité.

# Besoins
- Un devis en moins de cinq minutes, avec les prix fournisseurs à jour.
- Signature électronique et relances automatiques.
- **Point bloquant** : l'import de l'historique (huit ans de devis).

# Suite
- Démo sur leurs propres données dans dix jours.
- Proposition chiffrée après la démo.",
        mindmap: "mindmap
  root((Atelier Morel))
    Contexte
      Outil de devis à remplacer
      Douze utilisateurs
    Besoins
      Devis rapides
      Signature électronique
      Relances automatiques
    Risque
      Import de l'historique
    Suite
      Démo dans dix jours",
        actions: &[
            (
                "Préparer la démo avec un extrait de leurs devis",
                "Camille",
                "dans dix jours",
                false,
            ),
            (
                "Vérifier le format d'export de leur outil actuel",
                "Mehdi",
                "",
                true,
            ),
            (
                "Envoyer le compte rendu à Mme Morel",
                "Camille",
                "demain",
                false,
            ),
        ],
        speech: &[
            "Merci de nous recevoir. Pouvez-vous nous décrire comment vous faites vos devis aujourd'hui ?",
            "On a un vieil outil. Ça marche, mais chaque devis prend une demi-heure et les prix ne sont jamais à jour.",
            "Combien de personnes s'en servent ?",
            "Douze, dont quatre sur les chantiers, avec leur téléphone.",
            "Et l'historique, vous voulez le garder ?",
            "Oui, huit ans de devis. C'est le point bloquant pour nous.",
            "On peut faire une démo sur vos propres données, disons dans dix jours.",
            "Parfait, on vous envoie un export d'ici là.",
        ],
    },
    Demo {
        title: "Onboarding call with Priya",
        language: "en",
        minutes_ago: 26 * 60,
        seconds: 1083.0,
        source: Source::Device,
        state: State::Failed("provider: 429 rate limited, retry later (demo)"),
        summary: "",
        mindmap: "",
        actions: &[],
        speech: &[
            "Welcome aboard, Priya. Today is mostly about access and the first week.",
            "Thanks. I have my laptop, but I can't reach the staging environment yet.",
            "I'll open the request right after this call. You should have it by tomorrow.",
            "Your first task is small on purpose: fix a typo in the export screen, end to end.",
            "Sounds good. Who reviews it?",
            "Jonas. He is also your go-to person for the release process.",
        ],
    },
    Demo {
        title: "Voice memo",
        language: "en",
        minutes_ago: 2 * 24 * 60,
        seconds: 42.0,
        source: Source::Device,
        state: State::NoSpeech,
        summary: "",
        mindmap: "",
        actions: &[],
        speech: &[],
    },
    Demo {
        title: "Rétrospective sprint 14",
        language: "fr",
        minutes_ago: 3 * 24 * 60 + 90,
        seconds: 2300.0,
        source: Source::File,
        state: State::Done,
        summary: "# Ce qui a bien marché
- Les revues de code sous 24 h : **objectif tenu** sur tout le sprint.
- La nouvelle recette automatique a attrapé deux régressions.

# À améliorer
- Trop de sujets ouverts en parallèle : sept tickets en cours pour quatre personnes.
- Les estimations du lot import étaient deux fois trop basses.

# Décisions
- Limite de **deux tickets en cours** par personne.
- Un créneau de découpage chaque lundi matin.",
        mindmap: "mindmap
  root((Rétrospective sprint 14))
    Bien marché
      Revues sous 24 h
      Recette automatique
    À améliorer
      Trop de sujets en parallèle
      Estimations du lot import
    Décisions
      Deux tickets en cours
      Découpage le lundi",
        actions: &[
            (
                "Mettre la limite de tickets en cours dans le tableau",
                "Sofia",
                "lundi",
                true,
            ),
            (
                "Animer le premier créneau de découpage",
                "Mehdi",
                "lundi",
                false,
            ),
        ],
        speech: &[
            "On fait le tour : ce qui a bien marché, ce qu'on change. Sofia, tu commences ?",
            "Les revues de code sous vingt-quatre heures, on a tenu tout le sprint. C'est un vrai plus.",
            "En revanche on avait sept tickets en cours pour quatre personnes, et rien ne se terminait.",
            "Je propose une limite, deux tickets en cours par personne.",
            "Et pour les estimations du lot import, on était deux fois trop bas. Il faut découper avant.",
            "Un créneau de découpage le lundi matin, alors. Mehdi, tu animes le premier ?",
        ],
    },
    Demo {
        title: "Design review: onboarding flow",
        language: "en",
        minutes_ago: 4 * 24 * 60 + 200,
        seconds: 3167.0,
        source: Source::Device,
        state: State::Done,
        summary: "# Outcome
- **Variant B is kept**: one question per screen, progress shown at the top.
- The account step moves to the end, after the first import.

# Feedback
- The empty state needs an example, not only a button.
- Contrast of the secondary buttons is too low on the dark theme.
- **Open question**: do we ask for the team size at all?

# Next steps
- Prototype the new order and test it with five users.",
        mindmap: "mindmap
  root((Onboarding flow))
    Variant B
      One question per screen
      Progress at the top
    Changes
      Account step at the end
      Example in the empty state
    Open question
      Team size",
        actions: &[
            (
                "Prototype the new order of the steps",
                "Sofia",
                "Wednesday",
                true,
            ),
            ("Recruit five users for the test", "Tomás", "", true),
            (
                "Fix the contrast of the secondary buttons",
                "Jonas",
                "",
                false,
            ),
        ],
        speech: &[
            "Two variants on the table. A is the long form, B is one question per screen.",
            "B tested better: people finish it, and they know where they are thanks to the progress bar.",
            "I would move the account step to the end. Let them import something first.",
            "The empty state is a bit dry. Show an example, not only a button.",
            "And the secondary buttons are hard to read on the dark theme.",
            "Do we really need the team size? Let's leave that open and test without it.",
            "Sofia prototypes the new order, Tomás finds five users.",
        ],
    },
    Demo {
        title: "Idées pour l'atelier de mars",
        language: "fr",
        minutes_ago: 6 * 24 * 60 + 30,
        seconds: 375.0,
        source: Source::File,
        state: State::Done,
        summary: "# Idées
- Ouvrir par une **démo de dix minutes**, sans diapositives.
- Trois tables tournantes : import, recherche, partage.
- Finir par les questions, notées sur un mur.

# À prévoir
- Une salle pour trente personnes et un deuxième écran.",
        mindmap: "mindmap
  root((Atelier de mars))
    Démo de dix minutes
    Tables tournantes
      Import
      Recherche
      Partage
    Questions sur un mur
    Salle pour trente",
        actions: &[(
            "Réserver la salle et le deuxième écran",
            "",
            "fin février",
            false,
        )],
        speech: &[
            "Note pour l'atelier de mars. On ouvre par une démo de dix minutes, sans slides.",
            "Ensuite trois tables tournantes : import, recherche, partage.",
            "On finit par les questions, qu'on note sur un mur au fil de l'eau.",
            "Penser à réserver une salle pour trente personnes, avec un deuxième écran.",
        ],
    },
    Demo {
        title: "1:1 with Jonas",
        language: "en",
        minutes_ago: 8 * 24 * 60 + 15,
        seconds: 1650.0,
        source: Source::Device,
        state: State::Done,
        summary: "# Topics
- Jonas wants to **lead the next release** from start to end.
- On-call load was uneven in January: three nights for him, none for two others.

# Agreed
- He leads release 1.5, with Lina as backup.
- The on-call rota is rebuilt with a fixed order.",
        mindmap: "mindmap
  root((1:1 with Jonas))
    Release 1.5
      Jonas leads
      Lina as backup
    On-call
      Uneven in January
      Fixed order",
        actions: &[
            (
                "Rebuild the on-call rota with a fixed order",
                "Lina",
                "this month",
                false,
            ),
            ("Write down the release checklist", "Jonas", "", true),
        ],
        speech: &[
            "How was January for you?",
            "Busy. I had three on-call nights, and I think two people had none.",
            "That is not fair. I will rebuild the rota with a fixed order.",
            "Also, I would like to lead the next release, from the scope to the notes.",
            "Good idea. You take 1.5, and Lina stays as your backup.",
            "Then I'll start by writing down the checklist we all have in our heads.",
        ],
    },
];

type AnyError = Box<dyn std::error::Error>;

/// Seconds of silence in the demo audio: the same short file stands in for every recording.
const AUDIO_SECONDS: usize = 30;

/// Ogg CRC-32 (polynomial 0x04C11DB7, no reflection, initial value 0).
fn ogg_crc(page: &[u8]) -> u32 {
    page.iter().fold(0, |mut c, &b| {
        c ^= u32::from(b) << 24;
        for _ in 0..8 {
            c = if c & 0x8000_0000 != 0 {
                (c << 1) ^ 0x04C1_1DB7
            } else {
                c << 1
            };
        }
        c
    })
}

/// One Ogg page of the stream with serial 1: `packets` are whole packets, `granule` the position
/// after the last one, `kind` the header type flags.
fn ogg_page(seq: u32, kind: u8, granule: u64, packets: &[&[u8]]) -> Vec<u8> {
    let mut page = b"OggS\0".to_vec();
    page.push(kind);
    page.extend(granule.to_le_bytes());
    page.extend(1u32.to_le_bytes());
    page.extend(seq.to_le_bytes());
    page.extend([0; 4]); // CRC, filled below
    page.push(packets.len() as u8);
    // Packets here are far below 255 bytes: one segment each.
    page.extend(packets.iter().map(|p| p.len() as u8));
    page.extend(packets.concat());
    let crc = ogg_crc(&page);
    page[22..26].copy_from_slice(&crc.to_le_bytes());
    page
}

/// Ogg Opus of `AUDIO_SECONDS` of silence, built by hand so no encoder is needed: the headers
/// (mono, 48 kHz) then 20 ms packets of the 3-byte CELT silence frame. It passes `verify_ogg`.
fn silent_ogg() -> Vec<u8> {
    let mut head = b"OpusHead\x01\x01".to_vec();
    head.extend(312u16.to_le_bytes()); // pre-skip
    head.extend(48_000u32.to_le_bytes());
    head.extend([0; 3]); // gain, mapping family
    let mut tags = b"OpusTags".to_vec();
    tags.extend(0u32.to_le_bytes()); // no vendor string
    tags.extend(0u32.to_le_bytes()); // no comments
    let mut out = ogg_page(0, 0x02, 0, &[&head]);
    out.extend(ogg_page(1, 0, 0, &[&tags]));
    let silence: &[u8] = &[0xF8, 0xFF, 0xFE];
    let packets = AUDIO_SECONDS * 50;
    let per_page = 100;
    for (n, first) in (0..packets).step_by(per_page).enumerate() {
        let count = per_page.min(packets - first);
        let last = first + count == packets;
        out.extend(ogg_page(
            2 + n as u32,
            if last { 0x04 } else { 0 },
            ((first + count) * 960 + 312) as u64,
            &vec![silence; count],
        ));
    }
    out
}

#[tokio::main]
async fn main() -> Result<(), AnyError> {
    let home = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: demo_home <empty directory>")?,
    );
    std::fs::create_dir_all(&home)?;
    if std::fs::read_dir(&home)?.next().is_some() {
        return Err(format!(
            "{} is not empty: give a new or an empty directory",
            home.display()
        )
        .into());
    }

    // Long enough for the configuration loader, and plainly not secrets. Built here so secret
    // scanners do not flag this file.
    let fake = |n: char| format!("demo-home-not-a-secret-{}", n.to_string().repeat(16));
    let mut env = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(home.join(".env"))?;
    writeln!(
        env,
        "# Demo home made by the `demo_home` example. Nothing here is a real secret, and the\n\
         # providers point at a closed local port: no audio or text leaves the machine.\n\
         LIBERARR_SECRET_KEY={}\n\
         LIBERARR_API_KEY={}\n\
         LIBERARR_ADMIN_PASSWORD={}\n\
         LIBERARR_TRANSCRIBE_BASE_URL=http://127.0.0.1:9/v1\n\
         LIBERARR_TRANSCRIBE_API_KEY=demo-not-a-key\n\
         LIBERARR_TRANSCRIBE_MODEL=demo-transcribe\n\
         LIBERARR_LLM_BASE_URL=http://127.0.0.1:9/v1\n\
         LIBERARR_LLM_API_KEY=demo-not-a-key\n\
         LIBERARR_LLM_MODEL=demo-summarize",
        fake('0'),
        fake('1'),
        fake('2'),
    )?;
    FileKeyStore::new(home.join("device").join("keys.bin"), &fake('0')).save(&[
        UnlockMaterial {
            serial: SERIAL.into(),
            signature: Vec::new(),
            private_pem: String::new(),
            public_pem: String::new(),
            bind_token: String::new(),
        },
    ])?;

    // Where the app looks for the audio of a recording: `audio_path` is relative to this folder.
    std::fs::create_dir_all(home.join("audio"))?;
    let audio = silent_ogg();
    liberarr_core::audio::ogg::verify_ogg(&audio).map_err(|e| e.to_string())?;

    let db = Db::open(&home.join("liberarr.db"))?;
    // Marks this process as a live worker (see `Db::reset_stale`): while it runs, the app
    // leaves the job seeded as running alone, so one recording stays "processing".
    db.reset_stale().await?;
    let now = jiff::Timestamp::now();
    for d in DEMOS {
        let created_at = now.checked_sub(jiff::SignedDuration::from_mins(d.minutes_ago))?;
        std::fs::write(
            home.join("audio")
                .join(format!("demo-{}.ogg", d.minutes_ago)),
            &audio,
        )?;
        let id = db
            .add(NewRecording {
                title: Some(d.title.into()),
                source: d.source,
                source_ref: format!("demo-{}", d.minutes_ago),
                audio_path: format!("demo-{}.ogg", d.minutes_ago).into(),
                duration_s: Some(d.seconds),
                created_at,
            })
            .await?;
        if d.source == Source::Device {
            let session = created_at.as_second() as u32;
            db.record(&serial_hash(SERIAL), session, 1_000_000, id)
                .await?;
        }
        let step = d.seconds / d.speech.len().max(1) as f64;
        let transcript = Transcript {
            engine: "demo".into(),
            language: Some(d.language.into()),
            text: d.speech.join(" "),
            segments: d
                .speech
                .iter()
                .enumerate()
                .map(|(n, text)| Segment {
                    start: n as f64 * step,
                    end: (n + 1) as f64 * step,
                    text: (*text).into(),
                })
                .collect(),
            has_timestamps: true,
        };
        TranscriptRepo::save(&db, id, &transcript).await?;
        if matches!(d.state, State::Done) {
            let summary = MeetingSummary {
                title: d.title.into(),
                summary_md: d.summary.into(),
                mindmap: d.mindmap.into(),
                actions: d
                    .actions
                    .iter()
                    .enumerate()
                    .map(|(n, (action, owner, due, done))| Action {
                        action: (*action).into(),
                        owner: Some((*owner).to_string()).filter(|o| !o.is_empty()),
                        due: Some((*due).to_string()).filter(|d| !d.is_empty()),
                        start: Some((n as f64 + 0.5) * d.seconds / d.actions.len() as f64),
                        done: *done,
                    })
                    .collect(),
            };
            SummaryRepo::save(&db, id, &summary).await?;
        }

        // The jobs, taken and ended one by one as the worker does, up to the last step this
        // recording reaches.
        let last = if matches!(d.state, State::Done) {
            STEPS[2]
        } else {
            STEPS[1]
        };
        db.enqueue(id, STEPS[0]).await?;
        loop {
            let job = db.claim_next().await?.ok_or("the queue is empty")?;
            if job.step != last {
                db.complete(&job, next_step(&job.step)).await?;
                continue;
            }
            match d.state {
                State::Processing => {}
                State::Failed(error) => db.fail(&job, error).await?,
                State::Done | State::NoSpeech => db.complete(&job, None).await?,
            }
            break;
        }
    }

    println!(
        "Demo home ready: {} fictional recordings in {}",
        DEMOS.len(),
        home.display()
    );
    println!(
        "In another terminal: LIBERARR_HOME={} cargo run -p liberarr -- tui",
        home.display()
    );
    println!(
        "Keep this one open: it stands in for a running worker, so the recording shown as \
         processing stays in that state. Press Enter to stop."
    );
    std::io::stdin().read_line(&mut String::new())?;
    Ok(())
}
