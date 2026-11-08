mod assets;
mod auth;
mod config;
#[path = "../../shared/creator_objects.rs"]
mod creator_objects;
#[path = "../../shared/creator_state.rs"]
mod creator_state;
mod db;
mod http;
mod realtime;
mod world;

use config::Config;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub struct App {
    config: Config,
    db: Mutex<rusqlite::Connection>,
    oauth: Mutex<HashMap<String, auth::PendingLogin>>,
    grants: Mutex<HashMap<String, realtime::Grant>>,
    worlds: Mutex<world::Registry>,
    client: reqwest::Client,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::load()?;
    std::fs::create_dir_all(config.data.join("assets"))?;
    let db = db::open(&config.data.join("platform.sqlite"))?;
    let worlds = world::Registry {
        tps: config.tps,
        ..Default::default()
    };
    let app = Arc::new(App {
        config,
        db: Mutex::new(db),
        oauth: Mutex::new(HashMap::new()),
        grants: Mutex::new(HashMap::new()),
        worlds: Mutex::new(worlds),
        client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
    });
    let game = realtime::start(app.clone())?;
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
    Ok(())
}
