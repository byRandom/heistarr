//! End-to-end: the real app against fake TMDB, Torznab, qBittorrent,
//! Synology Download Station and Jellyfin servers started in the test.
//! Nothing leaves the machine.

use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use heistarr_server::{crypto::Cipher, db::Db, engine, routes, state};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Fake {
    /// Root of the fake download folder (what the client reports).
    downloads: PathBuf,
    /// qBittorrent: torrents added (url, tags), deleted hashes.
    added: Vec<(String, String)>,
    deleted: Vec<String>,
    /// State to report for added torrents.
    torrent_state: String,
    /// Synology: tasks created (uri, destination).
    ds_created: Vec<(String, String)>,
    ds_status: String,
    jellyfin_refreshes: usize,
}

type F = Arc<Mutex<Fake>>;

async fn serve(r: Router) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(l, r.into_make_service_with_connect_info::<SocketAddr>())
            .await
            .unwrap()
    });
    format!("http://{addr}")
}

fn tmdb() -> Router {
    let matrix = json!({"id": 603, "imdb_id": "tt0133093", "title": "The Matrix", "original_title": "The Matrix",
        "release_date": "1999-03-31", "overview": "A hacker learns the truth.", "poster_path": "/m.jpg"});
    let m2 = matrix.clone();
    Router::new()
        .route(
            "/3/movie/603",
            get(move || {
                let m = m2.clone();
                async move { Json(m) }
            }),
        )
        .route(
            "/3/search/movie",
            get(move || {
                let m = matrix.clone();
                async move { Json(json!({"results": [m]})) }
            }),
        )
        .route(
            "/3/configuration",
            get(
                |headers: HeaderMap, q: Query<HashMap<String, String>>| async move {
                    let ok = q.get("api_key").is_some_and(|k| k == "good-key")
                        || headers
                            .get("authorization")
                            .is_some_and(|v| v == "Bearer eyJgood");
                    if ok {
                        (StatusCode::OK, "{}")
                    } else {
                        (StatusCode::UNAUTHORIZED, "{}")
                    }
                },
            ),
        )
}

fn torznab() -> Router {
    Router::new().route("/api", get(|q: Query<HashMap<String, String>>| async move {
        if q.get("apikey").map(String::as_str) != Some("jackett-key") {
            return r#"<error code="100" description="Invalid API Key"/>"#.to_string();
        }
        match q.get("t").map(String::as_str) {
            Some("caps") => "<caps><searching/></caps>".to_string(),
            _ => r#"<rss xmlns:torznab="http://torznab.com/schemas/2015/feed"><channel>
<item><title>The.Matrix.1999.2160p.UHD.BluRay.x265-BIG</title><link>http://t/dl/uhd.torrent</link><size>60000000000</size><torznab:attr name="seeders" value="80"/></item>
<item><title>The.Matrix.1999.1080p.BluRay.x264-GOOD</title><link>http://t/dl/good.torrent</link><size>9000000000</size><torznab:attr name="seeders" value="50"/></item>
<item><title>The.Matrix.1999.720p.WEB-DL.x264-OK</title><link>http://t/dl/ok.torrent</link><size>3000000000</size><torznab:attr name="seeders" value="10"/></item>
</channel></rss>"#
                .to_string(),
        }
    }))
}

fn torrent_name(url: &str) -> &'static str {
    if url.contains("good") {
        "The.Matrix.1999.1080p.BluRay.x264-GOOD"
    } else {
        "The.Matrix.1999.720p.WEB-DL.x264-OK"
    }
}

fn qbit(f: F) -> Router {
    Router::new()
        .route("/api/v2/auth/login", post(|Form(b): Form<HashMap<String, String>>| async move {
            if b.get("password").map(String::as_str) == Some("qpass") {
                ([(header::SET_COOKIE, "SID=sid123; HttpOnly; path=/")], "Ok.").into_response()
            } else {
                "Fails.".into_response()
            }
        }))
        .route("/api/v2/app/version", get(|h: HeaderMap| async move {
            if h.get("cookie").is_some_and(|c| c == "SID=sid123") { (StatusCode::OK, "v5.0.0") } else { (StatusCode::FORBIDDEN, "") }
        }))
        .route("/api/v2/torrents/createCategory", post(|| async { StatusCode::OK }))
        .route("/api/v2/torrents/add", post(|State(f): State<F>, mut m: axum::extract::Multipart| async move {
            let (mut url, mut tags) = (String::new(), String::new());
            while let Some(field) = m.next_field().await.unwrap() {
                let name = field.name().unwrap_or("").to_string();
                let v = field.text().await.unwrap();
                match name.as_str() {
                    "urls" => url = v,
                    "tags" => tags = v,
                    _ => {}
                }
            }
            f.lock().unwrap().added.push((url, tags));
            "Ok."
        }))
        .route("/api/v2/torrents/info", get(|State(f): State<F>| async move {
            let f = f.lock().unwrap();
            let list: Vec<Value> = f
                .added
                .iter()
                .enumerate()
                .filter(|(i, _)| !f.deleted.contains(&format!("hash{i}")))
                .map(|(i, (url, tags))| {
                    let name = torrent_name(url);
                    json!({"hash": format!("hash{i}"), "name": name, "progress": 1.0, "state": f.torrent_state,
                           "content_path": format!("/remote/{name}"), "save_path": "/remote", "tags": tags})
                })
                .collect();
            Json(list)
        }))
        .route("/api/v2/torrents/delete", post(|State(f): State<F>, Form(b): Form<HashMap<String, String>>| async move {
            f.lock().unwrap().deleted.push(b["hashes"].clone());
            StatusCode::OK
        }))
        .with_state(f)
}

fn synology(f: F) -> Router {
    Router::new()
        .route("/webapi/query.cgi", get(|| async {
            Json(json!({"success": true, "data": {
                "SYNO.API.Auth": {"path": "entry.cgi", "minVersion": 1, "maxVersion": 7},
                "SYNO.DownloadStation.Task": {"path": "DownloadStation/task.cgi", "minVersion": 1, "maxVersion": 3},
                "SYNO.DownloadStation2.Task": {"path": "entry.cgi", "minVersion": 1, "maxVersion": 2}
            }}))
        }))
        .route("/webapi/entry.cgi", post(|State(f): State<F>, Form(b): Form<HashMap<String, String>>| async move {
            match b["api"].as_str() {
                "SYNO.API.Auth" if b["passwd"] == "dspass" => Json(json!({"success": true, "data": {"sid": "dssid"}})),
                "SYNO.API.Auth" => Json(json!({"success": false, "error": {"code": 400}})),
                "SYNO.DownloadStation2.Task" if b["_sid"] == "dssid" => {
                    let urls: Vec<String> = serde_json::from_str(&b["url"]).unwrap();
                    let dest: String = serde_json::from_str(&b["destination"]).unwrap();
                    let mut f = f.lock().unwrap();
                    f.ds_created.push((urls[0].clone(), dest));
                    let id = format!("dbid_{}", f.ds_created.len());
                    Json(json!({"success": true, "data": {"list_id": [], "task_id": [id]}}))
                }
                _ => Json(json!({"success": false, "error": {"code": 119}})),
            }
        }))
        .route("/webapi/DownloadStation/task.cgi", get(|State(f): State<F>, q: Query<HashMap<String, String>>| async move {
            assert_eq!(q["_sid"], "dssid");
            let f = f.lock().unwrap();
            let tasks: Vec<Value> = f
                .ds_created
                .iter()
                .enumerate()
                .map(|(i, (uri, dest))| json!({"id": format!("dbid_{}", i + 1), "title": torrent_name(uri), "size": 100,
                    "status": f.ds_status, "additional": {"detail": {"destination": dest, "uri": uri},
                    "transfer": {"size_downloaded": 100}}}))
                .collect();
            Json(json!({"success": true, "data": {"total": tasks.len(), "tasks": tasks}}))
        }).post(|| async { Json(json!({"success": true, "data": {}})) }))
        .with_state(f)
}

fn jellyfin(f: F) -> Router {
    Router::new()
        .route(
            "/Library/Refresh",
            post(|State(f): State<F>, h: HeaderMap| async move {
                assert_eq!(h["x-emby-token"], "jf-token");
                f.lock().unwrap().jellyfin_refreshes += 1;
                StatusCode::NO_CONTENT
            }),
        )
        .with_state(f)
}

struct Env {
    app: state::App,
    url: String,
    http: reqwest::Client,
    fake: F,
    dir: tempfile::TempDir,
    tmdb: String,
    torznab: String,
    qbit: String,
    synology: String,
    jellyfin: String,
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let downloads = dir.path().join("downloads");
    for name in [
        "The.Matrix.1999.1080p.BluRay.x264-GOOD",
        "The.Matrix.1999.720p.WEB-DL.x264-OK",
    ] {
        std::fs::create_dir_all(downloads.join(name)).unwrap();
        std::fs::write(downloads.join(name).join(format!("{name}.mkv")), b"video").unwrap();
        std::fs::write(downloads.join(name).join("sample.mkv"), b"s").unwrap();
    }
    let fake: F = Arc::new(Mutex::new(Fake {
        downloads,
        torrent_state: "uploading".into(),
        ds_status: "finished".into(),
        ..Default::default()
    }));
    let tmdb = serve(tmdb()).await;
    let cfg = state::Config {
        data_dir: dir.path().join("data"),
        web_dir: dir.path().join("web"),
        tmdb_api: format!("{tmdb}/3"),
        tmdb_images: format!("{tmdb}/img"),
        secure_cookie: false,
    };
    std::fs::create_dir_all(&cfg.data_dir).unwrap();
    let db = Db::open(&cfg.data_dir.join("heistarr.db"), Cipher::random()).unwrap();
    let app = state::AppState::new(db, cfg);
    let url = serve(routes::router(app.clone())).await;
    Env {
        app,
        url,
        http: reqwest::Client::builder()
            .cookie_store(true)
            .build()
            .unwrap(),
        torznab: serve(torznab()).await,
        qbit: serve(qbit(fake.clone())).await,
        synology: serve(synology(fake.clone())).await,
        jellyfin: serve(jellyfin(fake.clone())).await,
        tmdb,
        fake,
        dir,
    }
}

impl Env {
    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut r = self
            .http
            .request(method, format!("{}/api{path}", self.url))
            .header("X-Heistarr", "1");
        if let Some(b) = body {
            r = r.json(&b);
        }
        let r = r.send().await.unwrap();
        let st = StatusCode::from_u16(r.status().as_u16()).unwrap();
        let text = r.text().await.unwrap();
        (st, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.call(reqwest::Method::GET, path, None).await
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.call(reqwest::Method::POST, path, Some(body)).await
    }

    async fn put(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.call(reqwest::Method::PUT, path, Some(body)).await
    }

    fn lib(&self) -> PathBuf {
        self.dir.path().join("library/Movies")
    }

    /// Owner + settings + indexer, with the given download client.
    async fn configure(&self, client: &str) {
        let (st, _) = self
            .post(
                "/setup",
                json!({"username": "owner", "password": "password123"}),
            )
            .await;
        assert_eq!(st, StatusCode::OK);
        let (_, mut s) = self.get("/settings").await;
        let downloads = self.fake.lock().unwrap().downloads.clone();
        s["tmdb_key"] = json!("good-key");
        s["movies_root"] = json!(self.lib().to_string_lossy());
        s["download_client"] = json!(client);
        s["qbit_url"] = json!(self.qbit);
        s["qbit_username"] = json!("admin");
        s["qbit_password"] = json!("qpass");
        s["synology_url"] = json!(self.synology);
        s["synology_username"] = json!("heistarr");
        s["synology_password"] = json!("dspass");
        s["synology_destination"] = json!("data/torrents");
        s["remote_path"] = json!(if client == "synology" {
            "/data/torrents"
        } else {
            "/remote"
        });
        s["local_path"] = json!(downloads.to_string_lossy());
        s["media_server_kind"] = json!("jellyfin");
        s["media_server_url"] = json!(self.jellyfin);
        s["media_server_token"] = json!("jf-token");
        let (st, saved) = self.put("/settings", s).await;
        assert_eq!(st, StatusCode::OK, "{saved}");
        let (st, ix) = self
            .post("/indexers", json!({"id": 0, "name": "Jackett", "url": self.torznab, "api_key": "jackett-key", "enabled": true}))
            .await;
        assert_eq!(st, StatusCode::OK, "{ix}");
    }

    async fn add_matrix(&self) -> i64 {
        let (st, m) = self
            .post("/movies", json!({"tmdb_id": 603, "search_now": false}))
            .await;
        assert_eq!(st, StatusCode::CREATED, "{m}");
        m["id"].as_i64().unwrap()
    }
}

#[tokio::test]
async fn setup_login_and_csrf() {
    let e = env().await;
    let (_, s) = e.get("/setup").await;
    assert_eq!(s["needs_owner"], true);
    assert_eq!(e.get("/me").await.0, StatusCode::UNAUTHORIZED);

    // No CSRF header: refused.
    let r = e
        .http
        .post(format!("{}/api/setup", e.url))
        .json(&json!({"username": "x", "password": "password123"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 403);

    assert_eq!(
        e.post("/setup", json!({"username": "o", "password": "short"}))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        e.post(
            "/setup",
            json!({"username": "owner", "password": "password123"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        e.post(
            "/setup",
            json!({"username": "evil", "password": "password123"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(e.get("/me").await.1["username"], "owner");

    assert_eq!(e.post("/logout", json!({})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(e.get("/me").await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(e.get("/settings").await.0, StatusCode::UNAUTHORIZED);

    for _ in 0..5 {
        assert_eq!(
            e.post(
                "/login",
                json!({"username": "owner", "password": "wrong-password"})
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    // Rate limited now, even with the right password.
    assert_eq!(
        e.post(
            "/login",
            json!({"username": "owner", "password": "password123"})
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    e.app.limiter.clear(&["ip:127.0.0.1", "user:owner"]);
    assert_eq!(
        e.post(
            "/login",
            json!({"username": "owner", "password": "password123"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(e.get("/healthz").await.0, StatusCode::NOT_FOUND); // healthz is outside /api
    assert_eq!(
        e.http
            .get(format!("{}/healthz", e.url))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "ok"
    );
}

#[tokio::test]
async fn secrets_are_masked_and_kept() {
    let e = env().await;
    e.configure("qbittorrent").await;
    let (_, s) = e.get("/settings").await;
    assert_eq!(s["tmdb_key"], "••••-key");
    assert_eq!(s["qbit_password"], "••••pass");
    // Saving the masked values back keeps the real ones.
    assert_eq!(e.put("/settings", s.clone()).await.0, StatusCode::OK);
    let real = e.app.db.settings().unwrap();
    assert_eq!(real.tmdb_key, "good-key");
    assert_eq!(real.qbit_password, "qpass");
    let (_, ix) = e.get("/indexers").await;
    assert_eq!(ix[0]["api_key"], "••••-key");

    // Test buttons.
    assert_eq!(e.post("/settings/test/tmdb", s.clone()).await.1["ok"], true);
    assert_eq!(
        e.post("/settings/test/client", s.clone()).await.1["ok"],
        true,
        "qbit test"
    );
    assert_eq!(e.post("/indexers/test", ix[0].clone()).await.1["ok"], true);
    let mut bad = s.clone();
    bad["tmdb_key"] = json!("bad");
    assert_eq!(e.post("/settings/test/tmdb", bad).await.1["ok"], false);
    let mut bad = s.clone();
    bad["qbit_password"] = json!("nope");
    let (_, r) = e.post("/settings/test/client", bad).await;
    assert_eq!(r["ok"], false);
    assert!(
        r["message"]
            .as_str()
            .unwrap()
            .contains("wrong username or password"),
        "{r}"
    );
    let _ = &e.tmdb;
}

#[tokio::test]
async fn movie_end_to_end_with_qbittorrent() {
    let e = env().await;
    e.configure("qbittorrent").await;

    let (_, hits) = e.get("/catalog/search?q=matrix").await;
    assert_eq!(hits[0]["tmdb_id"], 603);
    assert_eq!(hits[0]["library_id"], Value::Null);

    let id = e.add_matrix().await;
    assert_eq!(
        e.post("/movies", json!({"tmdb_id": 603, "search_now": false}))
            .await
            .0,
        StatusCode::CONFLICT
    );

    // Interactive search: 1080p BluRay wins, 2160p rejected (above max).
    let (st, rel) = e.get(&format!("/movies/{id}/releases")).await;
    assert_eq!(st, StatusCode::OK, "{rel}");
    assert_eq!(rel[0]["title"], "The.Matrix.1999.1080p.BluRay.x264-GOOD");
    let uhd = rel
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["title"].as_str().unwrap().contains("2160p"))
        .unwrap();
    assert_eq!(uhd["score"], Value::Null);

    // Automatic search grabs it.
    let (st, grabbed) = e.post(&format!("/movies/{id}/search"), json!({})).await;
    assert_eq!(st, StatusCode::OK, "{grabbed}");
    assert_eq!(grabbed["title"], "The.Matrix.1999.1080p.BluRay.x264-GOOD");
    {
        let f = e.fake.lock().unwrap();
        assert_eq!(
            f.added,
            vec![(
                "http://t/dl/good.torrent".to_string(),
                "heistarr-1".to_string()
            )]
        );
    }
    let (_, m) = e.get(&format!("/movies/{id}")).await;
    assert_eq!(m["movie"]["status"], "downloading");
    // A second grab while one is active is refused.
    assert_eq!(
        e.post(&format!("/movies/{id}/search"), json!({})).await.0,
        StatusCode::BAD_GATEWAY
    );

    // The download checker imports it.
    engine::check_downloads(&e.app).await.unwrap();
    let dest = e.lib().join("The Matrix (1999)/The Matrix (1999).mkv");
    assert_eq!(std::fs::read(&dest).unwrap(), b"video");
    let (_, m) = e.get(&format!("/movies/{id}")).await;
    assert_eq!(m["movie"]["status"], "available", "{m}");
    assert_eq!(m["movie"]["file_quality"]["resolution"], "R1080");
    assert_eq!(m["downloads"][0]["state"], "imported");
    assert_eq!(e.fake.lock().unwrap().jellyfin_refreshes, 1);

    // History mentions it; nothing active any more.
    let (_, h) = e.get("/history").await;
    assert!(
        h.as_array()
            .unwrap()
            .iter()
            .any(|ev| ev["kind"] == "imported"),
        "{h}"
    );
    assert_eq!(e.get("/downloads").await.1, json!([]));

    // Already at the cutoff (BluRay-1080p): no upgrade is accepted.
    let (_, rel) = e.get(&format!("/movies/{id}/releases")).await;
    assert!(
        rel.as_array().unwrap().iter().all(|r| r["score"].is_null()),
        "{rel}"
    );

    // Library scan knows the file already.
    let (_, rep) = e.post("/library/scan", json!({})).await;
    assert_eq!(rep["already"], 1, "{rep}");
}

#[tokio::test]
async fn failed_download_is_blocklisted_and_replaced() {
    let e = env().await;
    e.configure("qbittorrent").await;
    let id = e.add_matrix().await;
    e.fake.lock().unwrap().torrent_state = "error".into();
    e.post(&format!("/movies/{id}/search"), json!({})).await;
    engine::check_downloads(&e.app).await.unwrap();

    let (_, m) = e.get(&format!("/movies/{id}")).await;
    assert_eq!(m["movie"]["status"], "wanted");
    assert_eq!(
        m["blocklist"][0]["release_title"],
        "The.Matrix.1999.1080p.BluRay.x264-GOOD"
    );
    assert_eq!(e.fake.lock().unwrap().deleted, vec!["hash0".to_string()]);

    // The next search takes the next best release.
    e.fake.lock().unwrap().torrent_state = "downloading".into();
    let (_, grabbed) = e.post(&format!("/movies/{id}/search"), json!({})).await;
    assert_eq!(grabbed["title"], "The.Matrix.1999.720p.WEB-DL.x264-OK");
}

#[tokio::test]
async fn movie_end_to_end_with_synology_download_station() {
    let e = env().await;
    e.configure("synology").await;
    let (_, s) = e.get("/settings").await;
    let (_, t) = e.post("/settings/test/client", s).await;
    assert_eq!(t["ok"], true, "{t}");

    let id = e.add_matrix().await;
    let (st, grabbed) = e.post(&format!("/movies/{id}/search"), json!({})).await;
    assert_eq!(st, StatusCode::OK, "{grabbed}");
    assert_eq!(
        e.fake.lock().unwrap().ds_created,
        vec![(
            "http://t/dl/good.torrent".to_string(),
            "data/torrents".to_string()
        )]
    );
    let (_, d) = e.get("/downloads").await;
    assert_eq!(d[0]["client_hash"], "dbid_1");

    // Still downloading: progress tracked, nothing imported.
    e.fake.lock().unwrap().ds_status = "downloading".into();
    engine::check_downloads(&e.app).await.unwrap();
    assert_eq!(e.get("/downloads").await.1[0]["state"], "downloading");

    // Finished: the NAS path /data/torrents/<title> maps to our folder.
    e.fake.lock().unwrap().ds_status = "finished".into();
    engine::check_downloads(&e.app).await.unwrap();
    let dest = e.lib().join("The Matrix (1999)/The Matrix (1999).mkv");
    assert!(dest.exists());
    assert_eq!(
        e.get(&format!("/movies/{id}")).await.1["movie"]["status"],
        "available"
    );

    let mut bad = e.get("/settings").await.1;
    bad["synology_password"] = json!("wrong");
    let (_, t) = e.post("/settings/test/client", bad).await;
    assert!(
        t["message"]
            .as_str()
            .unwrap()
            .contains("wrong username or password"),
        "{t}"
    );
}

#[tokio::test]
async fn library_scan_adds_existing_movies() {
    let e = env().await;
    e.configure("qbittorrent").await;
    let folder = e.lib().join("The Matrix (1999)");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("The.Matrix.1999.1080p.BluRay.x264.mkv"), b"v").unwrap();
    std::fs::create_dir_all(e.lib().join("Unknown Thing (2001)")).unwrap();
    std::fs::write(e.lib().join("Unknown Thing (2001)/x.mkv"), b"v").unwrap();

    let (st, rep) = e.post("/library/scan", json!({})).await;
    assert_eq!(st, StatusCode::OK, "{rep}");
    assert_eq!(rep["matched"].as_array().unwrap().len(), 1, "{rep}");
    assert_eq!(rep["unmatched"], json!(["Unknown Thing (2001)"]));
    let (_, movies) = e.get("/movies").await;
    assert_eq!(movies[0]["status"], "available");
    assert_eq!(movies[0]["file_quality"]["source"], "BluRay");
}

#[tokio::test]
async fn poster_proxy_rejects_bad_paths() {
    let e = env().await;
    e.configure("qbittorrent").await;
    assert_eq!(
        e.get("/poster/w342/..%2Fsecret").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(e.get("/poster/w9999/a.jpg").await.0, StatusCode::NOT_FOUND);
}
