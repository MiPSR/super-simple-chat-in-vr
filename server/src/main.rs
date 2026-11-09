mod asset_store;
mod assets;
mod auth;
mod config;
#[path = "../../shared/creator_objects.rs"]
mod creator_objects;
#[path = "../../shared/creator_state.rs"]
mod creator_state;
mod db;
mod http;
#[path = "../../shared/media_addresses.rs"]
mod media_addresses;
mod realtime;
mod uploads;
mod world;

use config::Config;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub struct App {
    asset_gate: Arc<tokio::sync::Mutex<()>>,
    uploads: Arc<uploads::Budget>,
    config: Config,
    db: Mutex<rusqlite::Connection>,
    oauth: Mutex<HashMap<String, auth::PendingLogin>>,
    grants: Mutex<HashMap<String, realtime::Grant>>,
    worlds: Mutex<world::Registry>,
    client: reqwest::Client,
    _data_lock: asset_store::DataDirLock,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::load()?;
    std::fs::create_dir_all(config.data.join("assets"))?;
    let _data_lock = asset_store::DataDirLock::acquire(&config.data)?;
    let (stale_uploads, stale_bytes) = uploads::recover(&config.data)?;
    if stale_uploads > 0 {
        eprintln!("Recovered {stale_uploads} abandoned uploads ({stale_bytes} bytes)");
    }
    let db = db::open(&config.data.join("platform.sqlite"))?;
    let worlds = world::Registry::with_tps(config.tps);
    let app = Arc::new(App {
        asset_gate: Arc::new(tokio::sync::Mutex::new(())),
        uploads: Arc::new(uploads::Budget::default()),
        config,
        db: Mutex::new(db),
        oauth: Mutex::new(HashMap::new()),
        grants: Mutex::new(HashMap::new()),
        worlds: Mutex::new(worlds),
        client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
        _data_lock,
    });
    let game = realtime::start(app.clone())?;
    let gc = asset_store::start(app.clone());
    let listener = tokio::net::TcpListener::bind(app.config.listen).await?;
    eprintln!(
        "Super Simple Chat in VR: {} (ENet {}, {} TPS)",
        app.config.public_url, app.config.enet, app.config.tps
    );
    axum::serve(listener, http::router(app))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    game.store(false, std::sync::atomic::Ordering::Relaxed);
    gc.abort();
    Ok(())
}
