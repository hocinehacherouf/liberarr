//! `liberarr serve`: the web UI and API, plus an in-process job worker. Never touches the device.
use super::Outcome;
use crate::container::App;
use liberarr_core::jobs::{JobQueue, run_job_once};
use liberarr_web::{AppState, Auth, AuthConfig, router};
use rust_i18n::t;
use std::net::{IpAddr, SocketAddr};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

const IDLE: Duration = Duration::from_secs(2);

pub async fn run(app: App, host: IpAddr, port: u16) -> Outcome {
    super::note_content_language(&app.cfg);
    super::note_model_download(&app);
    // Once, before the worker claims anything.
    let requeued = app.db.reset_stale().await?;
    if requeued > 0 {
        eprintln!("{}", t!("serve.requeued", count = requeued));
    }
    let app = Arc::new(app);
    let worker = Arc::clone(&app);
    tokio::spawn(async move {
        loop {
            match run_job_once(&worker.db, &*worker).await {
                Ok(true) => {}
                Ok(false) => tokio::time::sleep(IDLE).await,
                Err(e) => {
                    eprintln!("{}", t!("serve.worker_error", error = e));
                    tokio::time::sleep(IDLE).await;
                }
            }
        }
    });
    let auth = AuthConfig {
        username: "admin".into(),
        password: app.cfg.admin_password.clone(),
        api_key: app.cfg.api_key.clone(),
        secret_key: app.cfg.secret_key.clone(),
        cookie_secure: app.cfg.cookie_secure,
    };
    let state = AppState {
        backend: app,
        auth: Arc::new(Auth::new(auth)),
    };
    let listener = tokio::net::TcpListener::bind((host, port)).await?;
    let bind = listener.local_addr()?;
    println!("{}", t!("serve.listening", addr = bind));
    if !host.is_loopback() {
        println!("{}", t!("serve.network_warning"));
    }
    axum::serve(
        listener,
        router(state, bind).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(ExitCode::SUCCESS)
}
