//! Model files for local transcription and diarization: downloaded once into `<home>/models`, verified, then reused.
use crate::files::sha256_file;
use liberarr_core::{Error, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct ModelSpec {
    pub name: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// Pinned to a Hugging Face commit, so the URL is immutable.
pub const WHISPER_TURBO: ModelSpec = ModelSpec {
    name: "ggml-large-v3-turbo-q5_0.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo-q5_0.bin",
    size: 574_041_195,
    sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
};

pub const SILERO_VAD: ModelSpec = ModelSpec {
    name: "ggml-silero-v6.2.0.bin",
    url: "https://huggingface.co/ggml-org/whisper-vad/resolve/9ffd54a1e1ee413ddf265af9913beaf518d1639b/ggml-silero-v6.2.0.bin",
    size: 885_098,
    sha256: "2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987",
};

/// Speaker segmentation for diarization (pyannote segmentation-3.0, sherpa-onnx export; MIT, Copyright (c) 2022 CNRS).
pub const PYANNOTE_SEGMENTATION: ModelSpec = ModelSpec {
    name: "pyannote-segmentation-3.0.onnx",
    url: "https://huggingface.co/csukuangfj/sherpa-onnx-pyannote-segmentation-3-0/resolve/9403a6902bb58e3d5ae8c7e77c3422de279db2e0/model.onnx",
    size: 5_992_913,
    sha256: "220ad67ca923bef2fa91f2390c786097bf305bceb5e261d4af67b38e938e1079",
};

/// Speaker embedding for diarization (NVIDIA NeMo TitaNet-S; Apache-2.0).
pub const TITANET_SMALL: ModelSpec = ModelSpec {
    name: "nemo_en_titanet_small.onnx",
    url: "https://huggingface.co/csukuangfj/speaker-embedding-models/resolve/a8055d6daef723a6a4ecf4de233708575bf4994f/nemo_en_titanet_small.onnx",
    size: 40_257_283,
    sha256: "ad4a1802485d8b34c722d2a9d04249662f2ece5d28a7a039063ca22f515a789e",
};

const STALE_TMP: Duration = Duration::from_secs(15 * 60);

pub struct ModelStore {
    dir: PathBuf,
    http: reqwest::Client,
}

fn fail(m: &ModelSpec, why: &str) -> Error {
    Error::Provider(format!("model {}: {why}", m.name))
}

/// Drops `<name>.*.tmp` files left by a killed download. A live download keeps writing, so only files
/// untouched for a while go; errors are ignored.
fn remove_stale_tmp(dir: &Path, name: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let file = e.file_name();
        let file = file.to_string_lossy();
        if file.starts_with(&format!("{name}.")) && file.ends_with(".tmp") {
            let stale = e
                .metadata()
                .and_then(|md| md.modified())
                .is_ok_and(|t| t.elapsed().is_ok_and(|age| age > STALE_TMP));
            if stale {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

impl ModelStore {
    pub fn new(dir: PathBuf) -> Self {
        // Unlike `http::client()`, this one follows redirects (Hugging Face answers 302 to a CDN). It carries
        // no credentials. The read timeout bounds a stalled transfer, not the whole download.
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(60))
            .build()
            .expect("reqwest client");
        Self { dir, http }
    }

    pub fn is_present(&self, m: &ModelSpec) -> bool {
        fs::metadata(self.dir.join(m.name)).is_ok_and(|f| f.is_file() && f.len() == m.size)
    }

    /// Path of the verified model, downloading it once if missing.
    pub async fn ensure(&self, m: &ModelSpec) -> Result<PathBuf> {
        let path = self.dir.join(m.name);
        if self.is_present(m) {
            return Ok(path);
        }
        fs::create_dir_all(&self.dir).map_err(|_| fail(m, "cannot create the models directory"))?;
        remove_stale_tmp(&self.dir, m.name);
        // Per process: `serve` and the terminal app's worker may download at once.
        let tmp = self
            .dir
            .join(format!("{}.{}.tmp", m.name, std::process::id()));
        let done = match self.download(m, &tmp).await {
            Ok(()) => {
                fs::rename(&tmp, &path).map_err(|_| fail(m, "cannot move the file into place"))
            }
            Err(e) => Err(e),
        };
        if done.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        done.map(|()| path)
    }

    async fn download(&self, m: &ModelSpec, tmp: &Path) -> Result<()> {
        let mut resp = self
            .http
            .get(m.url)
            .send()
            .await
            .map_err(|_| fail(m, "download failed"))?;
        if !resp.status().is_success() {
            return Err(fail(
                m,
                &format!("download failed (HTTP {})", resp.status().as_u16()),
            ));
        }
        let mut file = fs::File::create(tmp).map_err(|_| fail(m, "cannot write the file"))?;
        let mut size = 0u64;
        while let Some(chunk) = resp.chunk().await.map_err(|_| fail(m, "download failed"))? {
            size += chunk.len() as u64;
            // Never more than the pinned size: a wrong server cannot fill the disk.
            if size > m.size {
                return Err(fail(m, "size mismatch"));
            }
            file.write_all(&chunk)
                .map_err(|_| fail(m, "cannot write the file"))?;
        }
        file.sync_all()
            .map_err(|_| fail(m, "cannot write the file"))?;
        drop(file);
        if size != m.size {
            return Err(fail(m, "size mismatch"));
        }
        let tmp = tmp.to_path_buf();
        let hash = tokio::task::spawn_blocking(move || sha256_file(&tmp))
            .await
            .map_err(|_| fail(m, "checksum failed"))?
            .map_err(|_| fail(m, "checksum failed"))?;
        if hash != m.sha256 {
            return Err(fail(m, "checksum mismatch"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        extract::State,
        http::{StatusCode, header},
        response::IntoResponse,
        routing::get,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const BODY: &[u8] = b"synthetic model bytes";

    fn sha256_hex(data: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// `/m` serves BODY and counts the requests; `/r` redirects to `/m`.
    async fn serve() -> (String, Arc<AtomicUsize>) {
        let hits = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/m",
                get(|State(h): State<Arc<AtomicUsize>>| async move {
                    h.fetch_add(1, Ordering::SeqCst);
                    BODY
                }),
            )
            .route(
                "/r",
                get(|| async { (StatusCode::FOUND, [(header::LOCATION, "/m")]).into_response() }),
            )
            .with_state(hits.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), hits)
    }

    fn spec(url: String, hash: &str) -> ModelSpec {
        ModelSpec {
            name: "test-model.bin",
            url: Box::leak(url.into_boxed_str()),
            size: BODY.len() as u64,
            sha256: Box::leak(hash.to_string().into_boxed_str()),
        }
    }

    fn store() -> (ModelStore, PathBuf) {
        // A counter: the clock is too coarse to tell parallel tests apart.
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "liberarr-models-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        (ModelStore::new(dir.clone()), dir)
    }

    #[tokio::test]
    async fn download_with_wrong_hash_leaves_nothing() {
        let (base, _) = serve().await;
        let (store, dir) = store();
        let m = spec(
            format!("{base}/m"),
            &sha256_hex(b"other bytes of the same size"),
        );
        let err = store.ensure(&m).await.unwrap_err().to_string();
        assert!(err.contains("test-model.bin"), "{err}");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn download_then_reuse() {
        let (base, hits) = serve().await;
        let (store, dir) = store();
        let m = spec(format!("{base}/m"), &sha256_hex(BODY));
        let path = store.ensure(&m).await.unwrap();
        assert_eq!(fs::read(&path).unwrap(), BODY);
        assert_eq!(store.ensure(&m).await.unwrap(), path);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn ensure_removes_only_stale_tmp_files() {
        let (base, _) = serve().await;
        let (store, dir) = store();
        fs::create_dir_all(&dir).unwrap();
        let stale = dir.join("test-model.bin.1.tmp");
        let fresh = dir.join("test-model.bin.2.tmp");
        fs::write(&fresh, b"live").unwrap();
        let f = fs::File::create(&stale).unwrap();
        f.set_modified(std::time::SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
        drop(f);
        let m = spec(format!("{base}/m"), &sha256_hex(BODY));
        store.ensure(&m).await.unwrap();
        assert!(!stale.exists());
        assert!(fresh.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn redirect_is_followed() {
        let (base, _) = serve().await;
        let (store, dir) = store();
        let m = spec(format!("{base}/r"), &sha256_hex(BODY));
        let path = store.ensure(&m).await.unwrap();
        assert_eq!(fs::read(&path).unwrap(), BODY);
        let _ = fs::remove_dir_all(&dir);
    }
}
