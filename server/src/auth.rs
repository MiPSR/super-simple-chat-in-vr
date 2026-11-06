use crate::{
    App, db,
    http::{ApiError, ApiResult},
};
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use subtle::ConstantTimeEq;

pub struct PendingLogin {
    pub expires: i64,
    pub browser_nonce: String,
}
#[derive(Clone, Serialize)]
pub struct Identity {
    pub id: String,
    pub admin: bool,
}
pub fn identity(app: &App, headers: &HeaderMap) -> ApiResult<Identity> {
    if let Some(value) = headers
        .get("authorization")
        .and_then(|s| s.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        && bool::from(value.as_bytes().ct_eq(app.config.admin_token.as_bytes()))
    {
        return Ok(Identity {
            id: "admin".into(),
            admin: true,
        });
    }
    let cookie = headers
        .get("cookie")
        .and_then(|s| s.to_str().ok())
        .unwrap_or("");
    let token = headers
        .get("authorization")
        .and_then(|s| s.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .or_else(|| {
            cookie
                .split(';')
                .find_map(|s| s.trim().strip_prefix("ssc_session="))
        })
        .ok_or_else(ApiError::unauthorized)?;
    let hash = db::digest(token);
    let db = app.db.lock().unwrap();
    let id = db.query_row("SELECT s.user_id FROM sessions s JOIN users u ON s.user_id=u.id WHERE s.hash=? AND s.expires>? AND u.enabled=1", rusqlite::params![hash, db::now()], |r| r.get(0)).map_err(|_| ApiError::unauthorized())?;
    Ok(Identity { id, admin: false })
}
pub fn admin(app: &App, headers: &HeaderMap) -> ApiResult<Identity> {
    let identity = identity(app, headers)?;
    if !identity.admin {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Administrator authentication required".into(),
        ));
    }
    Ok(identity)
}
pub fn ensure_admin_user(app: &App) -> ApiResult<()> {
    app.db.lock().unwrap().execute(
        "INSERT OR IGNORE INTO users(id,name) VALUES('admin','Administrator')",
        [],
    )?;
    Ok(())
}
pub fn session_valid(app: &App, id: &str, session: &str) -> bool {
    if id == "admin" && session == "admin" {
        return true;
    }
    let db = app.db.lock().unwrap();
    db.query_row("SELECT EXISTS(SELECT 1 FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.hash=? AND s.user_id=? AND s.expires>? AND u.enabled=1)", rusqlite::params![session,id,db::now()], |r| r.get(0)).unwrap_or(false)
}
pub fn cookie(app: &App, name: &str, value: &str, age: u32) -> String {
    format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={age}{}",
        if app.config.public_url.starts_with("https:") {
            "; Secure"
        } else {
            ""
        }
    )
}
pub fn begin(app: &App) -> ApiResult<(String, String)> {
    if app.config.discord_id.is_empty() {
        return Err(ApiError::bad("Discord OAuth is not configured"));
    }
    let state = db::token();
    let nonce = db::token();
    let mut pending = app.oauth.lock().unwrap();
    pending.retain(|_, p| p.expires > db::now());
    if pending.len() >= 256 {
        return Err(ApiError::bad("Too many pending logins"));
    }
    pending.insert(
        state.clone(),
        PendingLogin {
            expires: db::now() + 300,
            browser_nonce: db::digest(&nonce),
        },
    );
    let mut url = reqwest::Url::parse("https://discord.com/oauth2/authorize").unwrap();
    url.query_pairs_mut().extend_pairs([
        ("client_id", app.config.discord_id.as_str()),
        ("response_type", "code"),
        ("scope", "identify"),
        ("state", &state),
        (
            "redirect_uri",
            &format!("{}/api/v1/auth/callback", app.config.public_url),
        ),
    ]);
    Ok((url.to_string(), cookie(app, "ssc_oauth", &nonce, 300)))
}
#[derive(Deserialize)]
pub struct Callback {
    pub code: String,
    pub state: String,
}
pub async fn complete(app: Arc<App>, headers: HeaderMap, q: Callback) -> ApiResult<String> {
    let p = app
        .oauth
        .lock()
        .unwrap()
        .remove(&q.state)
        .ok_or_else(|| ApiError::bad("Invalid or consumed OAuth state"))?;
    let nonce = headers
        .get("cookie")
        .and_then(|s| s.to_str().ok())
        .unwrap_or("")
        .split(';')
        .find_map(|s| s.trim().strip_prefix("ssc_oauth="))
        .unwrap_or("");
    if p.expires < db::now()
        || !bool::from(
            db::digest(nonce)
                .as_bytes()
                .ct_eq(p.browser_nonce.as_bytes()),
        )
    {
        return Err(ApiError::unauthorized());
    }
    let response = app
        .client
        .post("https://discord.com/api/oauth2/token")
        .form(&[
            ("client_id", app.config.discord_id.as_str()),
            ("client_secret", app.config.discord_secret.as_str()),
            ("grant_type", "authorization_code"),
            ("code", q.code.as_str()),
            (
                "redirect_uri",
                &format!("{}/api/v1/auth/callback", app.config.public_url),
            ),
        ])
        .send()
        .await
        .map_err(ApiError::internal)?;
    if !response.status().is_success() {
        return Err(ApiError::bad("Discord rejected the login exchange"));
    }
    let token: serde_json::Value = response.json().await.map_err(ApiError::internal)?;
    let access = token["access_token"]
        .as_str()
        .ok_or_else(ApiError::unauthorized)?;
    let response = app
        .client
        .get("https://discord.com/api/users/@me")
        .bearer_auth(access)
        .send()
        .await
        .map_err(ApiError::internal)?;
    if !response.status().is_success() {
        return Err(ApiError::unauthorized());
    }
    let user: serde_json::Value = response.json().await.map_err(ApiError::internal)?;
    let id = user["id"].as_str().ok_or_else(ApiError::unauthorized)?;
    let db = app.db.lock().unwrap();
    if !db::allowed(&db, id) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Your Discord ID is not allowlisted".into(),
        ));
    }
    db.execute(
        "UPDATE users SET name=? WHERE id=?",
        rusqlite::params![user["username"].as_str().unwrap_or("User"), id],
    )?;
    let session = db::token();
    db.execute("DELETE FROM sessions WHERE expires<=?", [db::now()])?;
    db.execute(
        "INSERT INTO sessions VALUES(?,?,?)",
        rusqlite::params![db::digest(&session), id, db::now() + 86400],
    )?;
    Ok(cookie(&app, "ssc_session", &session, 86400))
}
