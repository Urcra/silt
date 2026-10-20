//! Accounts (ADR 0013): users with optional passwords and linked sign-in
//! identities, browser sessions, devices and the requests that sign them
//! in, and invites. Every secret is stored as a hash.

use std::path::Path;

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use silt_proto::{DevicePollReply, LoginReply};
use silt_types::DeviceId;
use sqlx::Row;
use sqlx::sqlite::SqlitePool;

use crate::{Server, ServerError, hash, id_bytes, id_from, now_ms, open_db};

const SESSION_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const DEVICE_IDLE_MS: i64 = 90 * 24 * 60 * 60 * 1000;
/// How stale `last_used_ms` may get before a request rewrites it, so most
/// requests don't write.
const DEVICE_USE_GRANULARITY_MS: i64 = 60 * 60 * 1000;
pub(crate) const DEVICE_REQUEST_TTL_MS: i64 = 15 * 60 * 1000;
const SIGN_IN_TTL_MS: i64 = 10 * 60 * 1000;
/// Letters that can't be misread for each other, for codes people type.
const CODE_LETTERS: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ";

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("user names are 1 to 39 lowercase letters, digits, dots, dashes or underscores, starting with a letter or digit")]
    BadName,
    #[error("a user named {0} already exists")]
    Taken(String),
    #[error("no user named {0}")]
    NoUser(String),
    #[error("passwords need at least 10 characters")]
    WeakPassword,
    #[error(transparent)]
    Server(#[from] ServerError),
}

impl From<sqlx::Error> for AccountError {
    fn from(error: sqlx::Error) -> Self {
        Self::Server(error.into())
    }
}

pub(crate) fn random_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

pub(crate) fn hash_password(password: &str) -> String {
    Argon2::default()
        .hash_password(password.as_bytes())
        .expect("hashing with a generated salt succeeds")
        .to_string()
}

pub(crate) fn verify_password(stored: &str, password: &str) -> bool {
    PasswordHash::new(stored).is_ok_and(|parsed| Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
}

pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 39
        && chars.next().is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '-' | '_'))
}

/// A user's name made valid: lowercased, other characters dropped.
pub(crate) fn name_from(text: &str) -> String {
    let name: String = text
        .to_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '-' | '_'))
        .skip_while(|ch| !ch.is_ascii_alphanumeric())
        .take(39)
        .collect();
    if name.is_empty() { "user".into() } else { name }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub admin: bool,
    pub has_password: bool,
    pub disabled: bool,
}

fn user_from(row: &sqlx::sqlite::SqliteRow) -> User {
    User {
        id: row.get("id"),
        name: row.get("name"),
        display_name: row.get("display_name"),
        email: row.get("email"),
        admin: row.get::<i64, _>("admin") != 0,
        has_password: row.get::<Option<String>, _>("password_hash").is_some(),
        disabled: row.get::<i64, _>("disabled") != 0,
    }
}

/// A static query selecting the columns `user_from` reads.
macro_rules! users_query {
    ($rest:literal) => {
        concat!("SELECT id, name, display_name, email, admin, password_hash, disabled FROM users ", $rest)
    };
}

pub(crate) async fn user_by_id(db: &SqlitePool, id: i64) -> Result<Option<User>, sqlx::Error> {
    Ok(sqlx::query(users_query!("WHERE id = ?"))
        .bind(id)
        .fetch_optional(db)
        .await?
        .as_ref()
        .map(user_from))
}

pub(crate) async fn user_by_name(db: &SqlitePool, name: &str) -> Result<Option<User>, sqlx::Error> {
    Ok(sqlx::query(users_query!("WHERE name = ?"))
        .bind(name)
        .fetch_optional(db)
        .await?
        .as_ref()
        .map(user_from))
}

pub(crate) async fn user_by_email(db: &SqlitePool, email: &str) -> Result<Option<User>, sqlx::Error> {
    Ok(sqlx::query(users_query!("WHERE lower(email) = lower(?)"))
        .bind(email)
        .fetch_optional(db)
        .await?
        .as_ref()
        .map(user_from))
}

pub(crate) struct NewUser<'a> {
    pub(crate) name: &'a str,
    pub(crate) display_name: Option<&'a str>,
    pub(crate) email: Option<&'a str>,
    pub(crate) password: Option<&'a str>,
    pub(crate) admin: bool,
}

pub(crate) async fn create_user(db: &SqlitePool, new: &NewUser<'_>) -> Result<i64, AccountError> {
    if !valid_name(new.name) {
        return Err(AccountError::BadName);
    }
    if new.password.is_some_and(|password| password.chars().count() < 10) {
        return Err(AccountError::WeakPassword);
    }
    let inserted = sqlx::query(
        "INSERT INTO users (name, display_name, email, password_hash, admin, created_ms) VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (name) DO NOTHING RETURNING id",
    )
    .bind(new.name)
    .bind(new.display_name)
    .bind(new.email)
    .bind(new.password.map(hash_password))
    .bind(i64::from(new.admin))
    .bind(now_ms())
    .fetch_optional(db)
    .await?;
    let id: i64 = inserted.ok_or_else(|| AccountError::Taken(new.name.to_owned()))?.get("id");
    crate::orgs::join_default_org(db, id).await?;
    Ok(id)
}

/// A free name based on `wanted`, adding a number when it is taken.
pub(crate) async fn free_name(db: &SqlitePool, wanted: &str) -> Result<String, sqlx::Error> {
    let base = name_from(wanted);
    for n in 0..1000 {
        let candidate = if n == 0 { base.clone() } else { format!("{base}-{n}") };
        if user_by_name(db, &candidate).await?.is_none() {
            return Ok(candidate);
        }
    }
    Ok(format!("{base}-{}", &uuid::Uuid::new_v4().simple().to_string()[..6]))
}

/// Checked whenever a name has no password, so the answer takes as long as
/// a real check and names can't be probed by timing.
const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2lsdHNhbHRzYWx0$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
/// Password checks running at once. Each takes ~19 MiB and a few dozen
/// milliseconds of CPU.
const CONCURRENT_CHECKS: usize = 4;
/// Failed sign-ins a name gets per window before it has to wait.
const FAILURES_ALLOWED: u32 = 10;
const FAILURE_WINDOW: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Keeps password checks from taking the server down: they run on blocking
/// threads, a few at a time, and a name that keeps failing waits.
pub(crate) struct PasswordChecks {
    running: tokio::sync::Semaphore,
    failures: std::sync::Mutex<std::collections::HashMap<String, (u32, std::time::Instant)>>,
}

impl Default for PasswordChecks {
    fn default() -> Self {
        Self {
            running: tokio::sync::Semaphore::new(CONCURRENT_CHECKS),
            failures: std::sync::Mutex::default(),
        }
    }
}

impl PasswordChecks {
    /// Whether attempts under `name` have to wait after too many failures.
    pub(crate) fn cooling_down(&self, name: &str) -> bool {
        let mut failures = self.failures.lock().expect("lock");
        failures.retain(|_, (_, since)| since.elapsed() < FAILURE_WINDOW);
        failures.get(name).is_some_and(|(count, _)| *count >= FAILURES_ALLOWED)
    }

    pub(crate) fn record(&self, name: &str, succeeded: bool) {
        let mut failures = self.failures.lock().expect("lock");
        if succeeded {
            failures.remove(name);
        } else {
            let entry = failures.entry(name.to_owned()).or_insert((0, std::time::Instant::now()));
            entry.0 += 1;
        }
    }

    async fn verify(&self, stored: String, password: String) -> bool {
        let Ok(_permit) = self.running.acquire().await else {
            return false;
        };
        tokio::task::spawn_blocking(move || verify_password(&stored, &password))
            .await
            .unwrap_or(false)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PasswordError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("too many failed sign-ins for this name; try again later")]
    TooManyFailures,
}

pub(crate) async fn check_password(server: &Server, name: &str, password: &str) -> Result<Option<User>, PasswordError> {
    let checks = &server.shared.passwords;
    if checks.cooling_down(name) {
        return Err(PasswordError::TooManyFailures);
    }
    let row = sqlx::query(users_query!("WHERE name = ?"))
        .bind(name)
        .fetch_optional(server.db())
        .await?;
    let stored: Option<String> = row.as_ref().and_then(|row| row.get("password_hash"));
    let matches = checks
        .verify(stored.clone().unwrap_or_else(|| DUMMY_HASH.to_owned()), password.to_owned())
        .await
        && stored.is_some();
    checks.record(name, matches);
    Ok(row.filter(|_| matches).map(|row| user_from(&row)).filter(|user| !user.disabled))
}

/// Registers a device of `user` and returns its token, shown only now.
pub(crate) async fn issue_device(db: &SqlitePool, user: i64, name: &str) -> Result<LoginReply, sqlx::Error> {
    let device = DeviceId(uuid::Uuid::new_v4().as_u128());
    let token = random_token();
    sqlx::query("INSERT INTO devices (id, name, token_hash, created_ms, last_used_ms, user_id) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(id_bytes(device.0))
        .bind(name)
        .bind(hash(&token).to_vec())
        .bind(now_ms())
        .bind(now_ms())
        .bind(user)
        .execute(db)
        .await?;
    let name = sqlx::query("SELECT name FROM users WHERE id = ?")
        .bind(user)
        .fetch_one(db)
        .await?
        .get("name");
    Ok(LoginReply {
        device,
        token,
        user: name,
    })
}

/// The device and user a device token stands for, unless the token lapsed
/// from disuse or its user is disabled.
pub(crate) async fn device_caller(db: &SqlitePool, token: &str) -> Result<Option<(DeviceId, i64)>, sqlx::Error> {
    let now = now_ms();
    let row = sqlx::query(
        "SELECT d.id, d.user_id, d.last_used_ms FROM devices d JOIN users u ON u.id = d.user_id
         WHERE d.token_hash = ? AND d.last_used_ms > ? AND u.disabled = 0",
    )
    .bind(hash(token).to_vec())
    .bind(now - DEVICE_IDLE_MS)
    .fetch_optional(db)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let device: Vec<u8> = row.get("id");
    if row.get::<i64, _>("last_used_ms") < now - DEVICE_USE_GRANULARITY_MS {
        sqlx::query("UPDATE devices SET last_used_ms = ? WHERE id = ?")
            .bind(now)
            .bind(&device)
            .execute(db)
            .await?;
    }
    Ok(Some((DeviceId(id_from(&device)), row.get("user_id"))))
}

/// A signed-in device, as its owner sees it.
pub(crate) struct DeviceInfo {
    pub(crate) id: DeviceId,
    pub(crate) name: String,
    pub(crate) created_ms: i64,
    pub(crate) last_used_ms: i64,
}

pub(crate) async fn devices_of(db: &SqlitePool, user: i64) -> Result<Vec<DeviceInfo>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, name, created_ms, last_used_ms FROM devices WHERE user_id = ? ORDER BY last_used_ms DESC")
        .bind(user)
        .fetch_all(db)
        .await?;
    Ok(rows
        .iter()
        .map(|row| DeviceInfo {
            id: DeviceId(id_from(row.get("id"))),
            name: row.get("name"),
            created_ms: row.get("created_ms"),
            last_used_ms: row.get("last_used_ms"),
        })
        .collect())
}

/// Signs out `device` if it belongs to `user`; returns whether it did.
pub(crate) async fn revoke_device(db: &SqlitePool, user: i64, device: DeviceId) -> Result<bool, sqlx::Error> {
    let mut tx = db.begin().await?;
    let revoked = sqlx::query("DELETE FROM devices WHERE id = ? AND user_id = ?")
        .bind(id_bytes(device.0))
        .bind(user)
        .execute(&mut *tx)
        .await?
        .rows_affected()
        == 1;
    sqlx::query("DELETE FROM leases WHERE device_id = ?")
        .bind(id_bytes(device.0))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(revoked)
}

/// Signs `user` out of every device and browser.
pub(crate) async fn sign_out_everywhere(db: &SqlitePool, user: i64) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    sqlx::query("DELETE FROM leases WHERE device_id IN (SELECT id FROM devices WHERE user_id = ?)")
        .bind(user)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM devices WHERE user_id = ?")
        .bind(user)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM web_sessions WHERE user_id = ?")
        .bind(user)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM device_requests WHERE user_id = ?")
        .bind(user)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

/// Signs `user` out of every browser except the one holding `keep`.
pub(crate) async fn end_other_sessions(db: &SqlitePool, user: i64, keep: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM web_sessions WHERE user_id = ? AND token_hash != ?")
        .bind(user)
        .bind(hash(keep).to_vec())
        .execute(db)
        .await?;
    Ok(())
}

pub(crate) async fn create_session(db: &SqlitePool, user: i64) -> Result<String, sqlx::Error> {
    let token = random_token();
    sqlx::query("INSERT INTO web_sessions (token_hash, user_id, expires_ms) VALUES (?, ?, ?)")
        .bind(hash(&token).to_vec())
        .bind(user)
        .bind(now_ms() + SESSION_TTL_MS)
        .execute(db)
        .await?;
    Ok(token)
}

pub(crate) async fn session_user(db: &SqlitePool, token: &str) -> Result<Option<User>, sqlx::Error> {
    let row = sqlx::query("SELECT user_id FROM web_sessions WHERE token_hash = ? AND expires_ms > ?")
        .bind(hash(token).to_vec())
        .bind(now_ms())
        .fetch_optional(db)
        .await?;
    match row {
        Some(row) => Ok(user_by_id(db, row.get("user_id")).await?.filter(|user| !user.disabled)),
        None => Ok(None),
    }
}

pub(crate) async fn end_session(db: &SqlitePool, token: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM web_sessions WHERE token_hash = ?")
        .bind(hash(token).to_vec())
        .execute(db)
        .await?;
    Ok(())
}

fn user_code() -> String {
    // Bytes past the last whole multiple of the alphabet's size are skipped,
    // so every letter is equally likely.
    let limit = 256 - 256 % CODE_LETTERS.len();
    let mut letters = String::new();
    while letters.len() < 8 {
        for byte in uuid::Uuid::new_v4().into_bytes() {
            if usize::from(byte) < limit && letters.len() < 8 {
                letters.push(char::from(CODE_LETTERS[usize::from(byte) % CODE_LETTERS.len()]));
            }
        }
    }
    format!("{}-{}", &letters[..4], &letters[4..])
}

pub(crate) fn normalize_user_code(code: &str) -> String {
    let letters: String = code.chars().filter(char::is_ascii_alphanumeric).map(|ch| ch.to_ascii_uppercase()).collect();
    if letters.len() == 8 {
        format!("{}-{}", &letters[..4], &letters[4..])
    } else {
        letters
    }
}

/// Starts a device sign-in; returns the device's secret and the code the
/// person sees.
pub(crate) async fn start_device(db: &SqlitePool, name: &str) -> Result<(String, String), sqlx::Error> {
    let device_code = random_token();
    let user_code = user_code();
    sqlx::query("INSERT INTO device_requests (code_hash, user_code, name, expires_ms) VALUES (?, ?, ?, ?)")
        .bind(hash(&device_code).to_vec())
        .bind(&user_code)
        .bind(name)
        .bind(now_ms() + DEVICE_REQUEST_TTL_MS)
        .execute(db)
        .await?;
    Ok((device_code, user_code))
}

/// The device name waiting under `user_code`, if the request is current.
pub(crate) async fn device_request(db: &SqlitePool, user_code: &str) -> Result<Option<String>, sqlx::Error> {
    Ok(sqlx::query("SELECT name FROM device_requests WHERE user_code = ? AND expires_ms > ? AND user_id IS NULL")
        .bind(normalize_user_code(user_code))
        .bind(now_ms())
        .fetch_optional(db)
        .await?
        .map(|row| row.get("name")))
}

pub(crate) async fn approve_device(db: &SqlitePool, user_code: &str, user: i64) -> Result<bool, sqlx::Error> {
    let updated = sqlx::query("UPDATE device_requests SET user_id = ? WHERE user_code = ? AND expires_ms > ? AND user_id IS NULL")
        .bind(user)
        .bind(normalize_user_code(user_code))
        .bind(now_ms())
        .execute(db)
        .await?
        .rows_affected();
    Ok(updated == 1)
}

/// Removes expired sign-in leftovers. Device requests and sign-in states
/// are created without an account, so they must not pile up.
pub(crate) async fn sweep_expired(db: &SqlitePool) -> Result<(), sqlx::Error> {
    let now = now_ms();
    for statement in [
        "DELETE FROM device_requests WHERE expires_ms <= ?",
        "DELETE FROM sign_in_states WHERE expires_ms <= ?",
        "DELETE FROM pairing_codes WHERE expires_ms <= ?",
        "DELETE FROM web_sessions WHERE expires_ms <= ?",
    ] {
        sqlx::query(statement).bind(now).execute(db).await?;
    }
    sqlx::query("DELETE FROM devices WHERE last_used_ms <= ?")
        .bind(now - DEVICE_IDLE_MS)
        .execute(db)
        .await?;
    sqlx::query("DELETE FROM leases WHERE device_id NOT IN (SELECT id FROM devices)")
        .execute(db)
        .await?;
    Ok(())
}

/// Refuses a waiting request; one already approved stays approved.
pub(crate) async fn deny_device(db: &SqlitePool, user_code: &str) -> Result<bool, sqlx::Error> {
    let denied = sqlx::query("DELETE FROM device_requests WHERE user_code = ? AND user_id IS NULL")
        .bind(normalize_user_code(user_code))
        .execute(db)
        .await?
        .rows_affected();
    Ok(denied == 1)
}

pub(crate) async fn poll_device(db: &SqlitePool, device_code: &str) -> Result<DevicePollReply, sqlx::Error> {
    // Taking an approved request and issuing its token happen once, however
    // many polls race for it.
    let approved = sqlx::query("DELETE FROM device_requests WHERE code_hash = ? AND user_id IS NOT NULL RETURNING name, user_id")
        .bind(hash(device_code).to_vec())
        .fetch_optional(db)
        .await?;
    if let Some(row) = approved {
        let name: String = row.get("name");
        return Ok(DevicePollReply::Approved {
            login: issue_device(db, row.get("user_id"), &name).await?,
        });
    }
    let waiting = sqlx::query("SELECT expires_ms FROM device_requests WHERE code_hash = ?")
        .bind(hash(device_code).to_vec())
        .fetch_optional(db)
        .await?;
    match waiting {
        Some(row) if row.get::<i64, _>("expires_ms") > now_ms() => Ok(DevicePollReply::Pending),
        _ => Ok(DevicePollReply::Gone),
    }
}

pub(crate) async fn create_invite(db: &SqlitePool, by: Option<i64>, org: Option<&str>, days: u32) -> Result<String, sqlx::Error> {
    let token = random_token();
    sqlx::query("INSERT INTO invites (token_hash, created_by, org, expires_ms) VALUES (?, ?, ?, ?)")
        .bind(hash(&token).to_vec())
        .bind(by)
        .bind(org)
        .bind(now_ms() + i64::from(days) * 24 * 60 * 60 * 1000)
        .execute(db)
        .await?;
    Ok(token)
}

/// An unused, current invite: `Some(org)` with the org it joins, if any.
pub(crate) async fn invite(db: &SqlitePool, token: &str) -> Result<Option<Option<String>>, sqlx::Error> {
    Ok(sqlx::query("SELECT org FROM invites WHERE token_hash = ? AND expires_ms > ? AND used_by IS NULL")
        .bind(hash(token).to_vec())
        .bind(now_ms())
        .fetch_optional(db)
        .await?
        .map(|row| row.get("org")))
}

/// An invite taken for one sign-up, until the account exists.
pub(crate) struct ClaimedInvite {
    token: String,
    org: Option<String>,
    expires_ms: i64,
}

/// Takes the invite for one sign-up: only one of two sign-ups racing for
/// it gets it. Complete it once the account exists, or release it.
pub(crate) async fn claim_invite(db: &SqlitePool, token: &str) -> Result<Option<ClaimedInvite>, sqlx::Error> {
    let Some(row) = sqlx::query("SELECT org, expires_ms FROM invites WHERE token_hash = ? AND expires_ms > ? AND used_by IS NULL")
        .bind(hash(token).to_vec())
        .bind(now_ms())
        .fetch_optional(db)
        .await?
    else {
        return Ok(None);
    };
    let expires_ms: i64 = row.get("expires_ms");
    let taken = sqlx::query("UPDATE invites SET expires_ms = 0 WHERE token_hash = ? AND expires_ms = ? AND used_by IS NULL")
        .bind(hash(token).to_vec())
        .bind(expires_ms)
        .execute(db)
        .await?
        .rows_affected();
    Ok((taken == 1).then(|| ClaimedInvite {
        token: token.to_owned(),
        org: row.get("org"),
        expires_ms,
    }))
}

impl ClaimedInvite {
    /// Marks the invite used by `user` and puts them in its org.
    pub(crate) async fn complete(self, db: &SqlitePool, user: i64) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE invites SET used_by = ? WHERE token_hash = ?")
            .bind(user)
            .bind(hash(&self.token).to_vec())
            .execute(db)
            .await?;
        if let Some(org) = &self.org {
            crate::orgs::add_member(db, org, user, false).await?;
        }
        Ok(())
    }

    /// Gives the invite back after a sign-up that failed.
    pub(crate) async fn release(self, db: &SqlitePool) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE invites SET expires_ms = ? WHERE token_hash = ? AND used_by IS NULL")
            .bind(self.expires_ms)
            .bind(hash(&self.token).to_vec())
            .execute(db)
            .await?;
        Ok(())
    }
}

pub(crate) async fn identity_user(db: &SqlitePool, provider: &str, subject: &str) -> Result<Option<i64>, sqlx::Error> {
    Ok(sqlx::query("SELECT user_id FROM identities WHERE provider = ? AND subject = ?")
        .bind(provider)
        .bind(subject)
        .fetch_optional(db)
        .await?
        .map(|row| row.get("user_id")))
}

pub(crate) async fn link_identity(db: &SqlitePool, provider: &str, subject: &str, user: i64) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT OR REPLACE INTO identities (provider, subject, user_id) VALUES (?, ?, ?)")
        .bind(provider)
        .bind(subject)
        .bind(user)
        .execute(db)
        .await?;
    Ok(())
}

/// A sign-in sent to a provider: its state's PKCE verifier, invite and
/// where to go afterwards.
pub(crate) struct SignInState {
    pub(crate) verifier: String,
    pub(crate) invite: Option<String>,
    pub(crate) next: String,
}

pub(crate) async fn save_sign_in(db: &SqlitePool, provider: &str, state: &str, saved: &SignInState) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO sign_in_states (state_hash, provider, verifier, invite, next, expires_ms) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(hash(state).to_vec())
        .bind(provider)
        .bind(&saved.verifier)
        .bind(&saved.invite)
        .bind(&saved.next)
        .bind(now_ms() + SIGN_IN_TTL_MS)
        .execute(db)
        .await?;
    Ok(())
}

/// The sign-in `state` names, used up by reading it.
pub(crate) async fn take_sign_in(db: &SqlitePool, provider: &str, state: &str) -> Result<Option<SignInState>, sqlx::Error> {
    Ok(sqlx::query(
        "DELETE FROM sign_in_states WHERE state_hash = ? AND provider = ? AND expires_ms > ? RETURNING verifier, invite, next",
    )
    .bind(hash(state).to_vec())
    .bind(provider)
    .bind(now_ms())
    .fetch_optional(db)
    .await?
    .map(|row| SignInState {
        verifier: row.get("verifier"),
        invite: row.get("invite"),
        next: row.get("next"),
    }))
}

/// Account administration on the server's host, without the server
/// running: `silt server user|invite`.
pub struct Admin {
    db: SqlitePool,
}

impl Admin {
    pub async fn open(data_dir: &Path) -> Result<Self, ServerError> {
        tokio::fs::create_dir_all(data_dir).await?;
        Ok(Self {
            db: open_db(data_dir).await?,
        })
    }

    pub async fn add_user(&self, name: &str, password: Option<&str>, admin: bool) -> Result<(), AccountError> {
        create_user(
            &self.db,
            &NewUser {
                name,
                display_name: None,
                email: None,
                password,
                admin,
            },
        )
        .await?;
        Ok(())
    }

    pub async fn users(&self) -> Result<Vec<User>, AccountError> {
        Ok(sqlx::query(users_query!("ORDER BY name"))
            .fetch_all(&self.db)
            .await?
            .iter()
            .map(user_from)
            .collect())
    }

    pub async fn set_password(&self, name: &str, password: &str) -> Result<(), AccountError> {
        if password.chars().count() < 10 {
            return Err(AccountError::WeakPassword);
        }
        let updated = sqlx::query("UPDATE users SET password_hash = ? WHERE name = ?")
            .bind(hash_password(password))
            .bind(name)
            .execute(&self.db)
            .await?
            .rows_affected();
        if updated == 0 {
            return Err(AccountError::NoUser(name.to_owned()));
        }
        Ok(())
    }

    pub async fn set_admin(&self, name: &str, admin: bool) -> Result<(), AccountError> {
        let updated = sqlx::query("UPDATE users SET admin = ? WHERE name = ?")
            .bind(i64::from(admin))
            .bind(name)
            .execute(&self.db)
            .await?
            .rows_affected();
        if updated == 0 {
            return Err(AccountError::NoUser(name.to_owned()));
        }
        Ok(())
    }

    /// Stops `name` signing in and signs them out everywhere, or lets them
    /// sign in again.
    pub async fn set_disabled(&self, name: &str, disabled: bool) -> Result<(), AccountError> {
        let user = user_by_name(&self.db, name).await?.ok_or_else(|| AccountError::NoUser(name.to_owned()))?;
        sqlx::query("UPDATE users SET disabled = ? WHERE id = ?")
            .bind(i64::from(disabled))
            .bind(user.id)
            .execute(&self.db)
            .await?;
        if disabled {
            sign_out_everywhere(&self.db, user.id).await?;
        }
        Ok(())
    }

    /// Signs `name` out of every device and browser, such as after a
    /// device is lost.
    pub async fn sign_out(&self, name: &str) -> Result<(), AccountError> {
        let user = user_by_name(&self.db, name).await?.ok_or_else(|| AccountError::NoUser(name.to_owned()))?;
        Ok(sign_out_everywhere(&self.db, user.id).await?)
    }

    /// An invite token for the sign-up page, optionally into `org`.
    pub async fn invite(&self, org: Option<&str>, days: u32) -> Result<String, AccountError> {
        Ok(create_invite(&self.db, None, org, days).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_codes_are_two_groups_of_four_letters() {
        for _ in 0..200 {
            let code = user_code();
            assert_eq!(code.len(), 9, "{code}");
            assert!(code.chars().enumerate().all(|(at, char)| if at == 4 { char == '-' } else { CODE_LETTERS.contains(&(char as u8)) }));
            assert_eq!(normalize_user_code(&code.to_lowercase()), code);
        }
    }

    #[test]
    fn passwords_hash_and_verify() {
        let stored = hash_password("correct horse battery");
        assert!(stored.starts_with("$argon2id$"));
        assert!(verify_password(&stored, "correct horse battery"));
        assert!(!verify_password(&stored, "wrong horse battery"));
        assert!(!verify_password("garbage", "x"));
    }

    #[test]
    fn names_and_codes_are_normalized() {
        assert!(valid_name("alice") && valid_name("a.b-c_9") && !valid_name("Alice") && !valid_name("-x") && !valid_name(""));
        assert_eq!(name_from("Ada Lovelace!"), "adalovelace");
        assert_eq!(name_from("__"), "user");
        let code = user_code();
        assert_eq!(code.len(), 9);
        assert_eq!(normalize_user_code(&code.to_lowercase().replace('-', " ")), code);
    }
}
