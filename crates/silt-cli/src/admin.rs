//! Accounts, orgs and the server's own administration (ADR 0013):
//! `silt auth`, `silt org`, `silt team`, `silt project` and the
//! `silt server` commands that work on its data dir.

use std::env;
use std::fs;
use std::io::{self, BufRead, IsTerminal};
use std::path::{Path, PathBuf};
use std::time::Duration;

use exn::{Exn, ResultExt};
use silt_daemon::paths;
use silt_daemon::remote::{self, Remote, RemoteConfig};
use silt_proto::{OrgChange, Visibility};
use silt_server::{Admin, Config};

use crate::CliError;
use crate::commands::{call, fail, is_running};

const SERVER_UNIT: &str = include_str!("../../../packaging/systemd/silt-server.service");

fn runtime() -> Result<tokio::runtime::Runtime, Exn<CliError>> {
    tokio::runtime::Runtime::new().or_raise(|| CliError("cannot start the async runtime".into()))
}

pub fn server_data(data: Option<PathBuf>) -> PathBuf {
    data.unwrap_or_else(|| paths::data_dir().join("server"))
}

/// A password from stdin when asked to or when stdin isn't a terminal,
/// else typed without echo.
fn read_password(prompt: &str, from_stdin: bool) -> Result<String, Exn<CliError>> {
    let password = if from_stdin || !io::stdin().is_terminal() {
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line).or_raise(|| CliError("cannot read the password".into()))?;
        line.trim_end_matches(['\r', '\n']).to_owned()
    } else {
        rpassword::prompt_password(prompt).or_raise(|| CliError("cannot read the password".into()))?
    };
    if password.is_empty() {
        return Err(fail("no password given"));
    }
    Ok(password)
}

fn generated_password() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..20].to_owned()
}

fn device_name(name: Option<String>) -> String {
    name.or_else(|| fs::read_to_string("/etc/hostname").ok().map(|name| name.trim().to_owned()))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "device".into())
}

pub struct InitOptions {
    pub data: Option<PathBuf>,
    pub listen: Option<String>,
    pub public_url: Option<String>,
    pub admin: String,
    pub password_stdin: bool,
    pub systemd: Option<PathBuf>,
    pub service_user: String,
    pub force: bool,
}

/// Writes `server.toml`, creates the first admin and, if asked, the
/// systemd unit.
pub fn server_init(options: InitOptions) -> Result<(), Exn<CliError>> {
    let data = server_data(options.data);
    let config_path = data.join(silt_server::config::CONFIG_FILE);
    if config_path.exists() && !options.force {
        return Err(fail(format!("{} exists; pass --force to replace it", config_path.display())));
    }
    let exe = env::current_exe().or_raise(|| CliError("cannot find the silt executable".into()))?;
    if options.systemd.is_some() && [Path::new("/home"), Path::new("/root")].iter().any(|home| exe.starts_with(home)) {
        return Err(fail(format!(
            "the unit would run {}, but it keeps the service out of home directories; install silt elsewhere first, for example `sudo install -m 755 {} /usr/local/bin/silt`, and run that",
            exe.display(),
            exe.display()
        )));
    }
    // The service user may own an existing directory, so only a new one is
    // made private here.
    silt_os::create_private_dir(&data).or_raise(|| CliError(format!("cannot create {}", data.display())))?;
    // Replacing a config keeps what these options don't set, sign-in
    // providers above all.
    let mut config = if config_path.exists() {
        Config::load(&data).or_raise(|| CliError(format!("cannot read {}", config_path.display())))?
    } else {
        Config::default()
    };
    if let Some(listen) = options.listen {
        config.listen = listen;
    }
    if let Some(url) = options.public_url {
        config.public_url = Some(url.trim_end_matches('/').to_owned());
    }
    if config.listen.parse::<std::net::SocketAddr>().is_err() {
        return Err(fail(format!("{} is not an address to listen on, such as 127.0.0.1:8090", config.listen)));
    }
    let password = if options.password_stdin { read_password("", true)? } else { generated_password() };
    // The admin is made first, which also checks the name and password, so
    // a refused one leaves no config behind.
    runtime()?
        .block_on(async {
            let admin = Admin::open(&data).await?;
            admin.add_user(&options.admin, Some(&password), true).await
        })
        .or_raise(|| CliError("cannot create the admin".into()))?;
    fs::write(&config_path, config.to_toml()).or_raise(|| CliError(format!("cannot write {}", config_path.display())))?;
    println!("wrote {}", config_path.display());
    println!("created admin {}", options.admin);
    if !options.password_stdin {
        println!("  password: {password}   (shown once; change it with `silt server user passwd {}`)", options.admin);
    }
    if let Some(unit) = options.systemd {
        hand_over(&data, &options.service_user);
        let text = SERVER_UNIT
            .replace("@SILT@", &exe.to_string_lossy())
            .replace("@DATA@", &data.to_string_lossy())
            .replace("@USER@", &options.service_user);
        fs::write(&unit, text).or_raise(|| CliError(format!("cannot write {}", unit.display())))?;
        println!(
            "wrote {}; start it with: systemctl daemon-reload && systemctl enable --now {}",
            unit.display(),
            unit.file_stem().map_or_else(String::new, |stem| stem.to_string_lossy().into_owned())
        );
    } else {
        println!("start it with: silt server run --data {}", data.display());
    }
    println!("then sign in from a machine with: silt auth login --server {}", config.public_url());
    Ok(())
}

/// Gives the data dir to the service's user, which runs the server: run as
/// root, init has made it root's.
fn hand_over(data: &Path, user: &str) {
    if !silt_os::is_root() {
        return;
    }
    let owner = format!("{user}:{user}");
    let given = std::process::Command::new("chown").arg("-R").arg(&owner).arg(data).status();
    if !given.is_ok_and(|status| status.success()) {
        eprintln!("silt: could not give {} to {owner}; do so before starting the service", data.display());
    }
}

pub fn server_run(listen: Option<String>, data: Option<PathBuf>) -> Result<(), Exn<CliError>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("SILT_LOG").unwrap_or_else(|_| "info".into()))
        .with_writer(io::stderr)
        .init();
    let data = server_data(data);
    let mut config = Config::load(&data).or_raise(|| CliError("cannot read the server's config".into()))?;
    if let Some(listen) = listen {
        config.listen = listen;
    }
    runtime()?.block_on(async {
        let listen = config.listen.clone();
        let server = silt_server::Server::open_with(&data, config)
            .await
            .or_raise(|| CliError(format!("cannot open server data in {}", data.display())))?;
        let listener =
            tokio::net::TcpListener::bind(&listen).await.or_raise(|| CliError(format!("cannot listen on {listen}")))?;
        eprintln!("silt server listening on {listen}, data in {}", data.display());
        server.serve(listener).await.or_raise(|| CliError("the server stopped".into()))
    })
}

pub fn server_pair(data: Option<PathBuf>, user: &str) -> Result<(), Exn<CliError>> {
    let data = server_data(data);
    let code = runtime()?
        .block_on(silt_server::create_pairing_code(&data, user))
        .or_raise(|| CliError("cannot create a pairing code".into()))?;
    println!("{code}");
    eprintln!("valid for 15 minutes; on the device run: silt auth login --server <url> --code {code}");
    Ok(())
}

fn admin(data: Option<PathBuf>) -> Result<(tokio::runtime::Runtime, Admin), Exn<CliError>> {
    let data = server_data(data);
    let runtime = runtime()?;
    let admin =
        runtime.block_on(Admin::open(&data)).or_raise(|| CliError(format!("cannot open server data in {}", data.display())))?;
    Ok((runtime, admin))
}

pub fn user_add(data: Option<PathBuf>, name: &str, is_admin: bool, password_stdin: bool) -> Result<(), Exn<CliError>> {
    let password = if password_stdin { read_password("", true)? } else { generated_password() };
    let (runtime, admin) = admin(data)?;
    runtime.block_on(admin.add_user(name, Some(&password), is_admin)).or_raise(|| CliError(format!("cannot add {name}")))?;
    println!("added {name}{}", if is_admin { " (admin)" } else { "" });
    if !password_stdin {
        println!("  password: {password}   (shown once)");
    }
    Ok(())
}

pub fn user_list(data: Option<PathBuf>) -> Result<(), Exn<CliError>> {
    let (runtime, admin) = admin(data)?;
    let users = runtime.block_on(admin.users()).or_raise(|| CliError("cannot list users".into()))?;
    for user in users {
        let mut notes = Vec::new();
        if user.admin {
            notes.push("admin");
        }
        if !user.has_password {
            notes.push("no password");
        }
        if user.disabled {
            notes.push("disabled");
        }
        println!("{:<20} {:<28} {}", user.name, user.email.unwrap_or_default(), notes.join(", "));
    }
    Ok(())
}

pub fn user_passwd(data: Option<PathBuf>, name: &str, password_stdin: bool) -> Result<(), Exn<CliError>> {
    let password = read_password(&format!("new password for {name}: "), password_stdin)?;
    let (runtime, admin) = admin(data)?;
    runtime.block_on(admin.set_password(name, &password)).or_raise(|| CliError(format!("cannot set {name}'s password")))?;
    println!("password changed");
    Ok(())
}

pub fn user_admin(data: Option<PathBuf>, name: &str, revoke: bool) -> Result<(), Exn<CliError>> {
    let (runtime, admin) = admin(data)?;
    runtime.block_on(admin.set_admin(name, !revoke)).or_raise(|| CliError(format!("cannot change {name}")))?;
    println!("{name} {} an admin", if revoke { "is no longer" } else { "is now" });
    Ok(())
}

pub fn user_disable(data: Option<PathBuf>, name: &str, disabled: bool) -> Result<(), Exn<CliError>> {
    let (runtime, admin) = admin(data)?;
    runtime.block_on(admin.set_disabled(name, disabled)).or_raise(|| CliError(format!("cannot change {name}")))?;
    if disabled {
        println!("{name} is disabled and signed out everywhere");
    } else {
        println!("{name} can sign in again");
    }
    Ok(())
}

pub fn user_sign_out(data: Option<PathBuf>, name: &str) -> Result<(), Exn<CliError>> {
    let (runtime, admin) = admin(data)?;
    runtime.block_on(admin.sign_out(name)).or_raise(|| CliError(format!("cannot sign {name} out")))?;
    println!("{name} is signed out of every device and browser");
    Ok(())
}

pub fn invite(data: Option<PathBuf>, org: Option<String>, days: u32) -> Result<(), Exn<CliError>> {
    let data = server_data(data);
    let config = Config::load(&data).or_raise(|| CliError("cannot read the server's config".into()))?;
    let (runtime, admin) = admin(Some(data))?;
    let token = runtime.block_on(admin.invite(org.as_deref(), days)).or_raise(|| CliError("cannot create an invite".into()))?;
    println!("{}/signup?invite={token}", config.public_url());
    eprintln!("valid for {days} days, for one person");
    Ok(())
}

pub fn backup(data: Option<PathBuf>, dest: &std::path::Path) -> Result<(), Exn<CliError>> {
    let data = server_data(data);
    let copied = runtime()?
        .block_on(silt_server::backup::backup(&data, dest))
        .or_raise(|| CliError(format!("could not back up {}", data.display())))?;
    println!("backed up {} to {} ({} MB)", data.display(), dest.display(), copied.bytes / 1_000_000);
    Ok(())
}

pub fn restore(backup: &std::path::Path, data: Option<PathBuf>, replace: bool) -> Result<(), Exn<CliError>> {
    let data = server_data(data);
    silt_server::backup::restore(backup, &data, replace)
        .or_raise(|| CliError(format!("could not restore into {}", data.display())))?;
    println!("restored {} into {}; start the server, and devices push back anything newer", backup.display(), data.display());
    Ok(())
}

fn save(config: &RemoteConfig) -> Result<(), Exn<CliError>> {
    remote::keep_sign_in(&paths::data_dir(), config).or_raise(|| CliError("could not save the server settings".into()))?;
    if is_running() {
        call::<String>(&silt_daemon::protocol::Request::Reload)?;
    }
    println!("signed in to {} as {}, device {} ({})", config.url, config.user, config.device, config.name);
    Ok(())
}

/// Tries to open `url` in a browser; the URL is printed either way.
fn open_browser(url: &str) {
    crate::commands::open_in_browser(url);
}

pub struct LoginOptions {
    pub server: String,
    pub code: Option<String>,
    pub user: Option<String>,
    pub password_stdin: bool,
    pub name: Option<String>,
    pub no_browser: bool,
}

/// Signs this machine in: with a pairing code, a password, or (by
/// default) by approving it in the browser.
pub fn auth_login(options: LoginOptions) -> Result<(), Exn<CliError>> {
    let name = device_name(options.name);
    let server = options.server.trim_end_matches('/').to_owned();
    let runtime = runtime()?;
    let config = if let Some(code) = options.code {
        runtime.block_on(remote::login(&server, &code, &name)).or_raise(|| CliError(format!("could not sign in to {server}")))?
    } else if let Some(user) = options.user {
        let password = read_password(&format!("password for {user}: "), options.password_stdin)?;
        runtime
            .block_on(remote::login_password(&server, &user, &password, &name))
            .or_raise(|| CliError(format!("could not sign in to {server}")))?
    } else {
        let started =
            runtime.block_on(remote::device_start(&server, &name)).or_raise(|| CliError(format!("could not reach {server}")))?;
        eprintln!("Open {} and approve this device.", started.verification_url);
        eprintln!("Check that the page shows the code {}.", started.user_code);
        if !options.no_browser {
            open_browser(&started.verification_url);
        }
        let waited = runtime.block_on(tokio::time::timeout(
            Duration::from_secs(started.expires_in_secs),
            remote::device_wait(&server, &name, &started.device_code, Duration::from_secs(started.interval_secs.max(1))),
        ));
        match waited {
            Ok(Ok(Some(config))) => config,
            Ok(Ok(None)) | Err(_) => return Err(fail("the sign-in was denied or expired; run `silt auth login` again")),
            Ok(Err(error)) => return Err(Exn::new(CliError(format!("could not sign in: {error}")))),
        }
    };
    save(&config)
}

fn remote_client() -> Result<Remote, Exn<CliError>> {
    remote::load_config(&paths::data_dir())
        .map(Remote::new)
        .ok_or_else(|| fail("not signed in to a server; use `silt auth login --server <url>`"))
}

/// Signs out of `server`, or of the default server.
pub fn auth_logout(server: Option<String>) -> Result<(), Exn<CliError>> {
    let data_dir = paths::data_dir();
    let config = match &server {
        Some(url) => remote::load_sign_ins(&data_dir)
            .into_iter()
            .find(|kept| kept.url == url.trim_end_matches('/'))
            .ok_or_else(|| fail(format!("not signed in to {url}")))?,
        None => remote_client()?.config,
    };
    let remote = Remote::new(config);
    if let Err(error) = runtime()?.block_on(remote.logout()) {
        eprintln!("silt: the server could not be told ({error}); forgetting the sign-in here anyway");
    }
    remote::forget_sign_in(&data_dir, &remote.config.url).or_raise(|| CliError("cannot remove the server settings".into()))?;
    if is_running() {
        call::<String>(&silt_daemon::protocol::Request::Reload)?;
    }
    println!("signed out of {}", remote.config.url);
    Ok(())
}

pub fn orgs() -> Result<(), Exn<CliError>> {
    let remote = remote_client()?;
    let reply = runtime()?.block_on(remote.orgs()).map_err(|error| fail(error.to_string()))?;
    for org in reply.orgs {
        println!("{} ({}; new projects default to {})", org.name, org.role, org.default_visibility);
        let members: Vec<String> =
            org.members
                .iter()
                .map(|member| {
                    if member.role == silt_proto::OrgRole::Owner {
                        format!("{} (owner)", member.user)
                    } else {
                        member.user.clone()
                    }
                })
                .collect();
        println!("  members: {}", members.join(", "));
        for team in &org.teams {
            println!("  team {}: {}", team.name, team.members.join(", "));
        }
        for project in &org.projects {
            println!("  project {project}");
        }
    }
    Ok(())
}

pub fn org_change(change: OrgChange) -> Result<(), Exn<CliError>> {
    let remote = remote_client()?;
    runtime()?.block_on(remote.org_change(&change)).map_err(|error| fail(error.to_string()))?;
    println!("done");
    Ok(())
}

/// The project at `cwd`: its origin remote, normalized.
pub fn project_here() -> Result<String, Exn<CliError>> {
    let cwd = env::current_dir().or_raise(|| CliError("cannot read the current directory".into()))?;
    let repo = silt_git::Repo::discover(&cwd).or_raise(|| CliError("not in a git repository".into()))?;
    repo.remote_url("origin")
        .ok()
        .flatten()
        .map(|url| silt_git::normalize_remote(&url))
        .ok_or_else(|| fail("this clone has no origin remote"))
}

pub fn parse_visibility(text: &str) -> Result<Visibility, String> {
    text.parse()
}

pub fn parse_on_off(text: &str) -> Result<bool, String> {
    match text {
        "on" | "true" | "yes" => Ok(true),
        "off" | "false" | "no" => Ok(false),
        _ => Err(format!("on or off, not {text:?}")),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn the_unit_template_names_every_placeholder() {
        for placeholder in ["@SILT@", "@DATA@", "@USER@"] {
            assert!(SERVER_UNIT.contains(placeholder));
        }
        assert!(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/systemd/silt-daemon.service").exists());
    }
}
