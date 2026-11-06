use crate::{App, assets, auth, db, realtime, world};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{delete, get, post, put},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{io::Read, sync::Arc};

#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);
pub type ApiResult<T> = Result<T, ApiError>;
impl ApiError {
    pub fn bad(s: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, s.into())
    }
    pub fn unauthorized() -> Self {
        Self(StatusCode::UNAUTHORIZED, "Authentication required".into())
    }
    pub fn internal(e: impl std::fmt::Display) -> Self {
        eprintln!("Request error: {e}");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal error; see server logs".into(),
        )
    }
}
impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        Self::internal(e)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":self.1}))).into_response()
    }
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../../web/index.html")) }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript")],
                    include_str!("../../web/app.js"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css")],
                    include_str!("../../web/style.css"),
                )
            }),
        )
        .route(
            "/health",
            get(|| async { Json(json!({"status":"ok","version":env!("CARGO_PKG_VERSION")})) }),
        )
        .route("/api/v1/auth/discord", get(oauth_begin))
        .route("/api/v1/auth/callback", get(oauth_callback))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/me", get(me))
        .route("/api/v1/preferences", put(preferences))
        .route("/api/v1/blocks/{target}", put(block))
        .route("/api/v1/assets", get(catalog).post(upload))
        .route("/api/v1/assets/{id}", delete(remove_asset))
        .route("/api/v1/assets/{id}/download", get(download))
        .route("/api/v1/instances", get(instances))
        .route("/api/v1/instances/{id}/invite", post(invite))
        .route("/api/v1/game-grants", post(game_grant))
        .route("/api/v1/admin/users", get(users).post(allow_user))
        .route(
            "/api/v1/admin/users/{id}",
            put(update_user).delete(revoke_user),
        )
        .route("/api/v1/admin/health", get(admin_health))
        .layer(DefaultBodyLimit::max(1_000_100_000))
        .layer(middleware::from_fn_with_state(app.clone(), security))
        .with_state(app)
}
async fn security(
    State(app): State<Arc<App>>,
    req: axum::extract::Request,
    next: Next,
) -> Response {
    if !matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD
    ) {
        if let Some(origin) = req.headers().get(header::ORIGIN) {
            if origin.to_str().ok() != Some(app.config.public_url.as_str()) {
                return ApiError(
                    StatusCode::FORBIDDEN,
                    "Cross-origin writes are forbidden".into(),
                )
                .into_response();
            }
        } else if req
            .headers()
            .get("sec-fetch-site")
            .is_some_and(|s| s != "same-origin" && s != "none")
        {
            return ApiError(
                StatusCode::FORBIDDEN,
                "Cross-site writes are forbidden".into(),
            )
            .into_response();
        }
    }
    let mut response = next.run(req).await;
    for (key, value) in [
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("cache-control", "no-store"),
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    ] {
        response.headers_mut().insert(
            axum::http::HeaderName::from_static(key),
            axum::http::HeaderValue::from_static(value),
        );
    }
    response
}
async fn oauth_begin(State(app): State<Arc<App>>) -> ApiResult<Response> {
    let (url, cookie) = auth::begin(&app)?;
    Ok(([(header::SET_COOKIE, cookie)], Redirect::to(&url)).into_response())
}
async fn oauth_callback(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(q): Query<auth::Callback>,
) -> ApiResult<Response> {
    let cookie = auth::complete(app, headers, q).await?;
    Ok(([(header::SET_COOKIE, cookie)], Redirect::to("/")).into_response())
}
async fn logout(State(app): State<Arc<App>>, headers: HeaderMap) -> ApiResult<Response> {
    let who = auth::identity(&app, &headers)?;
    app.db
        .lock()
        .unwrap()
        .execute("DELETE FROM sessions WHERE user_id=?", [&who.id])?;
    Ok((
        [(header::SET_COOKIE, auth::cookie(&app, "ssc_session", "", 0))],
        Json(json!({"ok":true})),
    )
        .into_response())
}
async fn me(State(app): State<Arc<App>>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let who = auth::identity(&app, &headers)?;
    if who.admin {
        auth::ensure_admin_user(&app)?;
    }
    let db = app.db.lock().unwrap();
    Ok(Json(
        json!({"user":who,"preferences":db::preferences(&db,&who.id),"protocol":1,"tps":app.config.tps}),
    ))
}
async fn preferences(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(p): Json<db::Preferences>,
) -> ApiResult<Json<Value>> {
    let who = auth::identity(&app, &headers)?;
    if !p.valid() {
        return Err(ApiError::bad("Invalid preferences"));
    }
    app.db.lock().unwrap().execute(
        "UPDATE users SET preferences=? WHERE id=?",
        rusqlite::params![serde_json::to_string(&p).unwrap(), who.id],
    )?;
    app.worlds.lock().unwrap().settings_changed(&who.id);
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
struct Flags {
    flags: u32,
}
async fn block(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(target): Path<String>,
    Json(f): Json<Flags>,
) -> ApiResult<Json<Value>> {
    let who = auth::identity(&app, &headers)?;
    if f.flags > 31 || target == who.id {
        return Err(ApiError::bad("Invalid block flags/target"));
    }
    app.db.lock().unwrap().execute("INSERT INTO blocks VALUES(?,?,?) ON CONFLICT(owner,target) DO UPDATE SET flags=excluded.flags",rusqlite::params![who.id,target,f.flags])?;
    let mut worlds = app.worlds.lock().unwrap();
    worlds.settings_changed(&who.id);
    worlds.settings_changed(&target);
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize, Default)]
struct Search {
    #[serde(default)]
    q: String,
    #[serde(default)]
    kind: String,
}
async fn catalog(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(q): Query<Search>,
) -> ApiResult<Json<Value>> {
    let who = auth::identity(&app, &headers)?;
    let db = app.db.lock().unwrap();
    let mut stmt=db.prepare("SELECT id,owner,kind,name,description,version,enabled FROM assets WHERE (enabled=1 OR ?1) AND (kind='world' OR owner=?2 OR ?1) AND (?3='' OR kind=?3) AND instr(lower(name || ' ' || description),lower(?4))>0 ORDER BY created DESC LIMIT 200")?;
    let rows=stmt.query_map(rusqlite::params![who.admin,who.id,q.kind,q.q],|r|Ok(json!({"id":r.get::<_,String>(0)?,"owner":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"name":r.get::<_,String>(3)?,"description":r.get::<_,String>(4)?,"version":r.get::<_,String>(5)?,"enabled":r.get::<_,bool>(6)?})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(Json(json!(rows)))
}
#[derive(Deserialize)]
struct UploadQuery {
    kind: String,
    replace: Option<String>,
}
async fn upload(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(q): Query<UploadQuery>,
    mut multipart: Multipart,
) -> ApiResult<Json<Value>> {
    let who = auth::identity(&app, &headers)?;
    if who.admin {
        auth::ensure_admin_user(&app)?;
    }
    if !matches!(q.kind.as_str(), "world" | "avatar") {
        return Err(ApiError::bad("kind must be world or avatar"));
    }
    let (budget, slots) = {
        let db = app.db.lock().unwrap();
        let sql = if q.kind == "world" {
            "SELECT world_bytes,world_slots FROM users WHERE id=?"
        } else {
            "SELECT avatar_bytes,avatar_slots FROM users WHERE id=?"
        };
        db.query_row(sql, [&who.id], |r| {
            Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64))
        })?
    };
    // Unique staging file; validation and publication run off the async executor.
    let temp = app.config.data.join(format!("upload-{}.tmp", db::token()));
    let result=async {
        let mut file=tokio::fs::File::create(&temp).await.map_err(ApiError::internal)?;
        let mut size=0u64;
        let mut field=multipart.next_field().await.map_err(|_|ApiError::bad("Invalid multipart upload"))?.ok_or_else(||ApiError::bad("Missing package file"))?;
        use tokio::io::AsyncWriteExt;
        while let Some(chunk)=field.chunk().await.map_err(|_|ApiError::bad("Invalid upload stream"))? {
            size+=chunk.len() as u64;
            if size>budget {return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE,format!("Package exceeds {budget} bytes")));}
            file.write_all(&chunk).await.map_err(ApiError::internal)?;
        }
        file.sync_all().await.map_err(ApiError::internal)?;
        drop(file);
        let app=app.clone(); let temp=temp.clone();
        tokio::task::spawn_blocking(move || {
            let mut f=std::fs::File::open(&temp).map_err(ApiError::internal)?;
            let manifest=assets::validate(&mut f,&q.kind,budget)?;
            use sha2::{Digest,Sha256}; use std::io::{Seek,SeekFrom};
            f.seek(SeekFrom::Start(0)).map_err(ApiError::internal)?;
            let mut hash=Sha256::new();
            let mut buf=[0u8;65536];
            loop {let n=f.read(&mut buf).map_err(ApiError::internal)?;if n==0 {break;}hash.update(&buf[..n]);}
            let version=hex::encode(hash.finalize());
            let mut db=app.db.lock().unwrap();
            let tx=db.transaction()?;
            let id=if let Some(id)=q.replace {
                let (owner,kind):(String,String)=tx.query_row("SELECT owner,kind FROM assets WHERE id=?",[&id],|r|Ok((r.get(0)?,r.get(1)?)))?;
                if (owner!=who.id && !who.admin) || kind!=q.kind {return Err(ApiError(StatusCode::FORBIDDEN,"Not your asset or wrong kind".into()));} id
            } else {
                let count:i64=tx.query_row("SELECT COUNT(*) FROM assets WHERE owner=? AND kind=?",rusqlite::params![who.id,q.kind],|r|r.get(0))?;
                if count as u64>=slots {return Err(ApiError::bad("Asset slots exhausted"));} db::token()[..16].to_string()
            };
            let destination=app.config.data.join("assets").join(format!("{version}.zip"));
            if !destination.exists() {std::fs::rename(&temp,&destination).map_err(ApiError::internal)?;}
            tx.execute("INSERT OR IGNORE INTO asset_versions VALUES(?,?,?)",rusqlite::params![version,serde_json::to_string(&manifest).unwrap(),size as i64])?;
            tx.execute("INSERT INTO assets(id,owner,kind,name,description,version,created) VALUES(?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,description=excluded.description,version=excluded.version,enabled=1",rusqlite::params![id,who.id,q.kind,manifest.name,manifest.description,version,db::now()])?;
            tx.commit()?;
            Ok(Json(json!({"id":id,"version":version,"manifest":manifest})))
        }).await.map_err(ApiError::internal)?
    }.await;
    let _ = tokio::fs::remove_file(temp).await;
    result
}
async fn remove_asset(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let who = auth::identity(&app, &headers)?;
    let n = app.db.lock().unwrap().execute(
        "DELETE FROM assets WHERE id=? AND (owner=? OR ?)",
        rusqlite::params![id, who.id, who.admin],
    )?;
    if n == 0 {
        return Err(ApiError::bad("Asset not found or not yours"));
    }
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
struct Version {
    version: Option<String>,
}
async fn download(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<Version>,
) -> ApiResult<Response> {
    let who = auth::identity(&app, &headers)?;
    use rusqlite::OptionalExtension;
    let catalog: Option<(String, String, String)> = app
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT kind,owner,version FROM assets WHERE id=? AND enabled=1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let version = q
        .version
        .or_else(|| catalog.as_ref().map(|a| a.2.clone()))
        .ok_or_else(|| ApiError::bad("Specify the live asset version"))?;
    let kind = catalog.as_ref().map(|a| a.0.as_str()).unwrap_or("");
    let live = {
        let registry = app.worlds.lock().unwrap();
        registry.instances.values().any(|i| {
            i.players.contains_key(&who.id)
                && ((i.world == id && i.version == version)
                    || i.players.values().any(|p| {
                        p.avatar.as_deref() == Some(&id)
                            && p.avatar_version.as_deref() == Some(&version)
                            && !db::blocked(&app.db.lock().unwrap(), &who.id, &p.id, 8)
                    }))
        })
    };
    let listed = catalog.as_ref().is_some_and(|(_, owner, current)| {
        (kind == "world" || owner == &who.id) && &version == current
    });
    if !who.admin && !(listed || live) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Asset version not available to you".into(),
        ));
    }
    if version.len() != 64 || !version.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::bad("Invalid version hash"));
    }
    let file = tokio::fs::File::open(
        app.config
            .data
            .join("assets")
            .join(format!("{version}.zip")),
    )
    .await
    .map_err(ApiError::internal)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip"),
            (header::CONTENT_DISPOSITION, "attachment"),
        ],
        Body::from_stream(tokio_util::io::ReaderStream::new(file)),
    )
        .into_response())
}
async fn instances(State(app): State<Arc<App>>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let who = auth::identity(&app, &headers)?;
    Ok(Json(app.worlds.lock().unwrap().list(&who.id)))
}
#[derive(Deserialize)]
struct Invite {
    user: String,
}
async fn invite(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(q): Json<Invite>,
) -> ApiResult<Json<Value>> {
    let who = auth::identity(&app, &headers)?;
    if !db::allowed(&app.db.lock().unwrap(), &q.user) {
        return Err(ApiError::bad("Invitee not authorized"));
    }
    let mut worlds = app.worlds.lock().unwrap();
    let i = worlds
        .instances
        .get_mut(&id)
        .ok_or_else(|| ApiError::bad("Instance gone"))?;
    if i.owner != who.id {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Only the instance creator may invite".into(),
        ));
    }
    i.invitations.retain(|_, (_, t)| *t > db::now());
    if i.invitations.len() >= 128 {
        return Err(ApiError::bad("Invitation limit"));
    }
    let token = db::token();
    i.invitations
        .insert(token.clone(), (q.user, db::now() + 300));
    Ok(Json(json!({"invitation":token,"expires_in":300})))
}
async fn game_grant(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(join): Json<world::Join>,
) -> ApiResult<Json<Value>> {
    let who = auth::identity(&app, &headers)?;
    if who.admin {
        auth::ensure_admin_user(&app)?;
    }
    if ![1, 2, 4].contains(&join.send_divisor) || ![1, 2, 4].contains(&join.receive_divisor) {
        return Err(ApiError::bad("Invalid network rate"));
    }
    let (world, version, manifest) = if join.instance.is_empty() {
        let (version,manifest):(String,String)=app.db.lock().unwrap().query_row("SELECT a.version,v.manifest FROM assets a JOIN asset_versions v ON a.version=v.hash WHERE a.id=? AND a.kind='world' AND a.enabled=1",[&join.world],|r|Ok((r.get(0)?,r.get(1)?)))?;
        (join.world.clone(), version, manifest)
    } else {
        let worlds = app.worlds.lock().unwrap();
        let i = worlds
            .instances
            .get(&join.instance)
            .ok_or_else(|| ApiError::bad("Instance gone"))?;
        (
            i.world.clone(),
            i.version.clone(),
            serde_json::to_string(&i.manifest).unwrap(),
        )
    };
    let manifest: assets::Manifest = serde_json::from_str(&manifest).map_err(ApiError::internal)?;
    let mut zip = zip::ZipArchive::new(
        std::fs::File::open(
            app.config
                .data
                .join("assets")
                .join(format!("{version}.zip")),
        )
        .map_err(ApiError::internal)?,
    )
    .map_err(ApiError::internal)?;
    let mut sources = Vec::new();
    for s in manifest.scripts.iter().filter(|s| s.side == "server") {
        let mut text = String::new();
        zip.by_name(&s.path)
            .map_err(ApiError::internal)?
            .take(65537)
            .read_to_string(&mut text)
            .map_err(ApiError::internal)?;
        sources.push(text);
    }
    let name =
        app.db
            .lock()
            .unwrap()
            .query_row("SELECT name FROM users WHERE id=?", [&who.id], |r| {
                r.get::<_, String>(0)
            })?;
    let ticket = db::token();
    let secret = db::token();
    let game_token = db::token();
    let game_session = db::digest(&game_token);
    app.db.lock().unwrap().execute(
        "INSERT INTO sessions VALUES(?,?,?)",
        rusqlite::params![game_session, who.id, db::now() + 86400],
    )?;
    let mut grants = app.grants.lock().unwrap();
    grants.retain(|_, g| g.expires > db::now());
    if grants.len() >= 256 {
        return Err(ApiError::bad("Pending connection limit"));
    }
    grants.insert(
        ticket.clone(),
        realtime::Grant {
            user: who.id.clone(),
            session: game_session,
            name,
            join: world::Join { world, ..join },
            version,
            manifest,
            sources,
            secret: hex::decode(&secret).unwrap(),
            expires: db::now() + 60,
        },
    );
    Ok(Json(
        json!({"ticket":ticket,"secret":secret,"api_token":game_token,"api_url":app.config.public_url,"user":who.id,"expires_in":60,"port":app.config.enet.port(),"protocol":1,"tps":app.config.tps}),
    ))
}
#[derive(Deserialize)]
struct User {
    id: String,
}
async fn allow_user(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(q): Json<User>,
) -> ApiResult<Json<Value>> {
    auth::admin(&app, &headers)?;
    if !(5..=24).contains(&q.id.len()) || !q.id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ApiError::bad("Expected a Discord snowflake ID"));
    }
    app.db.lock().unwrap().execute(
        "INSERT INTO users(id) VALUES(?) ON CONFLICT(id) DO UPDATE SET enabled=1",
        [q.id],
    )?;
    Ok(Json(json!({"ok":true})))
}
async fn users(State(app): State<Arc<App>>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    auth::admin(&app, &headers)?;
    let db = app.db.lock().unwrap();
    let mut s = db.prepare(
        "SELECT id,name,enabled,world_slots,avatar_slots,world_bytes,avatar_bytes FROM users",
    )?;
    let rows=s.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"enabled":r.get::<_,bool>(2)?,"world_slots":r.get::<_,u32>(3)?,"avatar_slots":r.get::<_,u32>(4)?,"world_bytes":r.get::<_,i64>(5)?,"avatar_bytes":r.get::<_,i64>(6)?})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(Json(json!(rows)))
}
async fn revoke_user(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    auth::admin(&app, &headers)?;
    let db = app.db.lock().unwrap();
    db.execute("UPDATE users SET enabled=0 WHERE id=?", [&id])?;
    db.execute("DELETE FROM sessions WHERE user_id=?", [id])?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
struct Quotas {
    world_slots: u32,
    avatar_slots: u32,
    world_bytes: u64,
    avatar_bytes: u64,
}
async fn update_user(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(q): Json<Quotas>,
) -> ApiResult<Json<Value>> {
    auth::admin(&app, &headers)?;
    if q.world_slots > 1000
        || q.avatar_slots > 1000
        || q.world_bytes > 1_000_000_000
        || q.avatar_bytes > 1_000_000_000
    {
        return Err(ApiError::bad("Quota exceeds process resource ceiling"));
    }
    app.db.lock().unwrap().execute(
        "UPDATE users SET world_slots=?,avatar_slots=?,world_bytes=?,avatar_bytes=? WHERE id=?",
        rusqlite::params![
            q.world_slots,
            q.avatar_slots,
            q.world_bytes as i64,
            q.avatar_bytes as i64,
            id
        ],
    )?;
    Ok(Json(json!({"ok":true})))
}
async fn admin_health(State(app): State<Arc<App>>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    auth::admin(&app, &headers)?;
    let w = app.worlds.lock().unwrap();
    Ok(Json(
        json!({"instances":w.instances.len(),"players":w.instances.values().map(|i|i.players.len()).sum::<usize>(),"tick":w.tick,"tps":app.config.tps}),
    ))
}
