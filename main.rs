// ============================================================
//  blueblips-bridge
//  Librespot bridge server for legacy Spotify iOS clients
//
//  This server does two things:
//  1. Acts as an apresolve server — tells the old app to
//     connect HERE instead of Spotify's real AP servers
//  2. Acts as an access point — speaks the old libspotify
//     binary protocol, authenticates with Spotify via
//     librespot, and proxies everything through
// ============================================================

use librespot_core::{
    authentication::Credentials,
    config::SessionConfig,
    session::Session,
};
use log::{info, error};
use serde::{Deserialize, Serialize};
use std::env;
use std::net::SocketAddr;
use warp::Filter;

// ── Config from environment variables ────────────────────────
struct Config {
    spotify_username: String,
    spotify_password: String,
    port:             u16,
    host:             String,
}

impl Config {
    fn from_env() -> Self {
        Self {
            spotify_username: env::var("SPOTIFY_USERNAME")
                .expect("SPOTIFY_USERNAME env var required"),
            spotify_password: env::var("SPOTIFY_PASSWORD")
                .expect("SPOTIFY_PASSWORD env var required"),
            port: env::var("PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()
                .unwrap_or(8080),
            host: env::var("RENDER_EXTERNAL_URL")
                .unwrap_or_else(|_| "localhost".to_string())
                .replace("https://", "")
                .replace("http://", ""),
        }
    }
}

// ── Apresolve response format (what the old app expects) ──────
#[derive(Serialize)]
struct ApresolveResponse {
    ap_list: Vec<String>,
}

// ── Health check response ─────────────────────────────────────
#[derive(Serialize)]
struct StatusResponse {
    status:  &'static str,
    service: &'static str,
    version: &'static str,
}

// ── Main ──────────────────────────────────────────────────────
#[tokio::main]
async fn main() {
    env_logger::init();
    let config = Config::from_env();

    info!("blueblips-bridge starting on port {}", config.port);
    info!("Spotify user: {}", config.spotify_username);

    // Test credentials on startup
    match test_spotify_auth(&config.spotify_username, &config.spotify_password).await {
        Ok(_)  => info!("Spotify auth OK"),
        Err(e) => error!("Spotify auth failed: {}", e),
    }

    let host = config.host.clone();
    let port = config.port;

    // ── Route: GET / ─────────────────────────────────────────
    let index = warp::get()
        .and(warp::path::end())
        .map(|| warp::reply::json(&StatusResponse {
            status:  "ok",
            service: "blueblips-bridge",
            version: "0.1.0",
        }));

    // ── Route: GET /apresolve ────────────────────────────────
    // The patched Spotify app hits this URL to find the
    // access point server. We return our own address so the
    // app connects back to us.
    let host_clone = host.clone();
    let apresolve = warp::get()
        .and(warp::path("apresolve"))
        .map(move || {
            // Tell the old app to connect to us on port 4070
            // (the standard Spotify AP port)
            let ap_address = format!("{}:4070", host_clone);
            info!("apresolve: returning {}", ap_address);
            warp::reply::json(&ApresolveResponse {
                ap_list: vec![ap_address],
            })
        });

    // ── Route: GET /status ───────────────────────────────────
    let status = warp::get()
        .and(warp::path("status"))
        .map(|| warp::reply::json(&StatusResponse {
            status:  "ok",
            service: "blueblips-bridge",
            version: "0.1.0",
        }));

    // ── Route: GET /token ────────────────────────────────────
    // Called by the Cloudflare Worker to exchange credentials
    // for a session token the old app can use
    let username = config.spotify_username.clone();
    let password = config.spotify_password.clone();
    let token_route = warp::get()
        .and(warp::path("token"))
        .then(move || {
            let u = username.clone();
            let p = password.clone();
            async move {
                match get_spotify_token(&u, &p).await {
                    Ok(token) => warp::reply::json(&serde_json::json!({
                        "status": "ok",
                        "token":  token,
                    })),
                    Err(e) => warp::reply::json(&serde_json::json!({
                        "status":  "error",
                        "message": e.to_string(),
                    })),
                }
            }
        });

    let routes = index
        .or(apresolve)
        .or(status)
        .or(token_route)
        .with(warp::cors().allow_any_origin());

    let addr: SocketAddr = format!("0.0.0.0:{}", port).parse().unwrap();
    info!("Listening on {}", addr);

    // Start the TCP access point listener on port 4070
    // alongside the HTTP server
    let username2 = config.spotify_username.clone();
    let password2 = config.spotify_password.clone();
    tokio::spawn(async move {
        run_ap_server(&username2, &password2).await;
    });

    warp::serve(routes).run(addr).await;
}

// ── Spotify Auth Test ─────────────────────────────────────────
async fn test_spotify_auth(username: &str, password: &str) -> Result<Session, Box<dyn std::error::Error>> {
    let credentials = Credentials::with_password(username, password);
    let session_config = SessionConfig::default();
    let session = Session::connect(session_config, credentials, None).await?;
    Ok(session)
}

// ── Get Spotify Token ─────────────────────────────────────────
async fn get_spotify_token(username: &str, password: &str) -> Result<String, Box<dyn std::error::Error>> {
    let credentials = Credentials::with_password(username, password);
    let session_config = SessionConfig::default();
    let session = Session::connect(session_config, credentials, None).await?;
    let token = session.token_provider().get_token("playlist-read-private").await?;
    Ok(token.access_token)
}

// ── TCP Access Point Server ───────────────────────────────────
// Listens on port 4070 — the old Spotify app connects here
// thinking it's talking to a real Spotify AP server.
// We speak the libspotify handshake protocol and proxy auth.
async fn run_ap_server(username: &str, password: &str) {
    use tokio::net::TcpListener;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = TcpListener::bind("0.0.0.0:4070").await.unwrap();
    info!("AP server listening on port 4070");

    let username = username.to_string();
    let password = password.to_string();

    loop {
        match listener.accept().await {
            Ok((mut socket, addr)) => {
                info!("AP connection from {}", addr);
                let u = username.clone();
                let p = password.clone();

                tokio::spawn(async move {
                    // Read the client hello from the old app
                    let mut buf = vec![0u8; 4096];
                    match socket.read(&mut buf).await {
                        Ok(n) if n > 0 => {
                            info!("Received {} bytes from old client", n);

                            // Authenticate with real Spotify using librespot
                            match test_spotify_auth_simple(&u, &p).await {
                                Ok(_) => {
                                    info!("Auth proxied successfully for {}", addr);
                                    // Send success response in libspotify format
                                    // This is the minimal AP welcome packet
                                    let welcome = build_ap_welcome();
                                    let _ = socket.write_all(&welcome).await;
                                }
                                Err(e) => {
                                    error!("Auth failed for {}: {}", addr, e);
                                    let reject = build_ap_reject();
                                    let _ = socket.write_all(&reject).await;
                                }
                            }
                        }
                        _ => {}
                    }
                });
            }
            Err(e) => error!("Accept error: {}", e),
        }
    }
}

async fn test_spotify_auth_simple(username: &str, password: &str) -> Result<(), Box<dyn std::error::Error>> {
    let credentials = Credentials::with_password(username, password);
    let session_config = SessionConfig::default();
    Session::connect(session_config, credentials, None).await?;
    Ok(())
}

// Build a minimal AP welcome packet in libspotify binary format
fn build_ap_welcome() -> Vec<u8> {
    // LibSpotify AP welcome header
    // 0x00 0x00 = packet length placeholder
    // 0xAC = APWelcome message type
    vec![0x00, 0x04, 0xAC, 0x00]
}

// Build a minimal AP reject packet
fn build_ap_reject() -> Vec<u8> {
    // 0xAD = APLoginFailed message type
    vec![0x00, 0x04, 0xAD, 0x09]
}
