//! Accounts, sessions, login rate limiting and CSRF protection.
//!
//! - Passwords: argon2id.
//! - Sessions: a random token in an HttpOnly, SameSite=Strict cookie; only
//!   its SHA-256 is stored.
//! - CSRF: every state-changing API call must carry the `X-Heistarr`
//!   header. Browsers cannot add custom headers to cross-site requests
//!   without a CORS preflight, which this server never allows.

use crate::db::User;
use crate::state::{App, AppError};
use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;

pub const COOKIE: &str = "heistarr_session";
pub const CSRF_HEADER: &str = "x-heistarr";
/// Sessions last 30 days.
pub const SESSION_AGE: i64 = 30 * 24 * 3600;

pub fn hash_password(pw: &str) -> anyhow::Result<String> {
    let mut salt = [0u8; 16];
    rand::rng().fill_bytes(&mut salt);
    let salt = SaltString::encode_b64(&salt).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(Argon2::default()
        .hash_password(pw.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .to_string())
}

pub fn verify_password(pw: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .is_ok_and(|h| Argon2::default().verify_password(pw.as_bytes(), &h).is_ok())
}

/// A hash to verify against when the user does not exist, so a wrong
/// username takes as long as a wrong password.
pub fn dummy_hash() -> &'static str {
    static H: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    H.get_or_init(|| hash_password("not-a-real-password").unwrap_or_default())
}

pub fn new_token() -> String {
    let mut b = [0u8; 32];
    rand::rng().fill_bytes(&mut b);
    hex::encode(b)
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn session_cookie(token: &str, secure: bool) -> String {
    format!(
        "{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={SESSION_AGE}{}",
        if secure { "; Secure" } else { "" }
    )
}

pub fn clear_cookie() -> String {
    format!("{COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0")
}

pub fn cookie_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|kv| {
            kv.trim()
                .strip_prefix(&format!("{COOKIE}="))
                .map(str::to_string)
        })
        .filter(|t| !t.is_empty())
}

pub fn validate_new_account(username: &str, password: &str) -> Result<(), AppError> {
    let u = username.trim();
    if u.is_empty() || u.chars().count() > 64 || u.chars().any(char::is_control) {
        return Err(AppError::bad("the username must be 1 to 64 characters"));
    }
    if password.chars().count() < 8 {
        return Err(AppError::bad("the password must be at least 8 characters"));
    }
    if password.len() > 1024 {
        return Err(AppError::bad("the password is too long"));
    }
    Ok(())
}

/// The signed-in user, for handlers that need one.
pub struct CurrentUser(pub User);

impl FromRequestParts<App> for CurrentUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, app: &App) -> Result<Self, AppError> {
        let unauth = || AppError::new(StatusCode::UNAUTHORIZED, "sign in first");
        let token = cookie_token(&parts.headers).ok_or_else(unauth)?;
        let user = app
            .db
            .session_user(&token_hash(&token), SESSION_AGE)?
            .ok_or_else(unauth)?;
        Ok(Self(user))
    }
}

/// The signed-in owner, for settings and other admin handlers.
pub struct Owner(#[allow(dead_code)] pub User);

impl FromRequestParts<App> for Owner {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, app: &App) -> Result<Self, AppError> {
        let CurrentUser(u) = CurrentUser::from_request_parts(parts, app).await?;
        if !u.is_owner {
            return Err(AppError::new(
                StatusCode::FORBIDDEN,
                "only the owner can do this",
            ));
        }
        Ok(Self(u))
    }
}

/// Rejects state-changing API requests without the CSRF header.
pub async fn csrf(State(_app): State<App>, req: Request, next: Next) -> Result<Response, AppError> {
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe && req.headers().get(CSRF_HEADER).is_none() {
        return Err(AppError::new(
            StatusCode::FORBIDDEN,
            "missing X-Heistarr header",
        ));
    }
    Ok(next.run(req).await)
}

/// Failed-login counter: at most `MAX_FAILS` failures per key in `WINDOW`
/// seconds. Keys are the client address and the username, separately.
#[derive(Default)]
pub struct Limiter {
    fails: Mutex<HashMap<String, Vec<i64>>>,
}

const MAX_FAILS: usize = 5;
const WINDOW: i64 = 15 * 60;

impl Limiter {
    pub fn blocked(&self, keys: &[&str]) -> bool {
        let now = crate::db::now();
        let mut m = self.fails.lock().unwrap_or_else(|e| e.into_inner());
        keys.iter().any(|k| {
            m.get_mut(*k).is_some_and(|v| {
                v.retain(|t| *t > now - WINDOW);
                v.len() >= MAX_FAILS
            })
        })
    }

    pub fn fail(&self, keys: &[&str]) {
        let now = crate::db::now();
        let mut m = self.fails.lock().unwrap_or_else(|e| e.into_inner());
        if m.len() > 10_000 {
            m.retain(|_, v| v.iter().any(|t| *t > now - WINDOW));
        }
        for k in keys {
            m.entry(k.to_string()).or_default().push(now);
        }
    }

    pub fn clear(&self, keys: &[&str]) {
        let mut m = self.fails.lock().unwrap_or_else(|e| e.into_inner());
        for k in keys {
            m.remove(*k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_roundtrip() {
        let h = hash_password("correct horse").unwrap();
        assert!(verify_password("correct horse", &h));
        assert!(!verify_password("wrong", &h));
        assert!(!verify_password("x", "garbage"));
    }

    #[test]
    fn limiter_blocks_after_five() {
        let l = Limiter::default();
        for _ in 0..4 {
            l.fail(&["ip:1", "user:a"]);
        }
        assert!(!l.blocked(&["ip:1"]));
        l.fail(&["ip:1", "user:a"]);
        assert!(l.blocked(&["ip:2", "user:a"]));
        l.clear(&["ip:1", "user:a"]);
        assert!(!l.blocked(&["ip:1", "user:a"]));
    }

    #[test]
    fn reads_cookie() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            "a=b; heistarr_session=tok; c=d".parse().unwrap(),
        );
        assert_eq!(cookie_token(&h).as_deref(), Some("tok"));
    }
}
