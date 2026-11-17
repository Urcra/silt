//! `silt sandbox-run`: starts a program in Silt's sandbox (ADR 0086), as the
//! plan the CLI or the daemon wrote for the run says.

use std::ffi::OsString;
use std::path::Path;

use exn::Exn;

use crate::CliError;
use crate::commands::fail;

#[cfg(target_os = "linux")]
pub fn run(plan_path: &Path, in_network: bool, command: &[OsString]) -> Result<(), Exn<CliError>> {
    use std::os::unix::process::CommandExt as _;

    use exn::ResultExt as _;
    use silt_os::sandbox;
    use silt_workspace::Network;

    let plan = sandbox::read_plan(plan_path).or_raise(|| CliError("cannot read the sandbox's plan".into()))?;
    let tools = sandbox::tools(plan.network).map_err(|error| fail(error.to_string()))?;
    let scratch = plan_path.parent().unwrap_or(Path::new("."));
    if plan.network == Network::Full && !in_network {
        // pasta makes the network namespace and runs this again inside it.
        let exe = std::env::current_exe().or_raise(|| CliError("cannot find the silt executable".into()))?;
        let mut again: Vec<OsString> =
            vec![exe.into(), "sandbox-run".into(), "--plan".into(), plan_path.into(), "--in-network".into(), "--".into()];
        again.extend(command.iter().cloned());
        let pasta = tools.pasta.as_ref().ok_or_else(|| fail(sandbox::SandboxError::NoPasta.to_string()))?;
        let (name, value) = sandbox::Ids::here().variable();
        let error = std::process::Command::new(pasta).args(sandbox::pasta_args(&again)).env(name, value).exec();
        return Err(fail(format!("cannot start pasta: {error}")));
    }
    let resolv = if in_network {
        sandbox::block_routes(&silt_workspace::subtract(&plan.blocked, &plan.allowed)).map_err(|error| {
            let mut said = error.to_string();
            if said.contains("not permitted") {
                said.push_str(
                    "; pasta couldn't give the sandbox a user namespace of its own, as when Silt runs as root in some containers. \
                     Run it as a normal user, or give the workspace another network (`silt workspace policy <name> --network none`)",
                );
            }
            fail(said)
        })?;
        let resolv = scratch.join("resolv.conf");
        std::fs::write(&resolv, sandbox::resolver_file()).or_raise(|| CliError("cannot write the sandbox's resolver".into()))?;
        Some(resolv)
    } else {
        None
    };
    let program = sandbox::seccomp_program().or_raise(|| CliError("cannot make the sandbox's seccomp filter".into()))?;
    let filter = scratch.join("seccomp.bpf");
    std::fs::write(&filter, program).or_raise(|| CliError("cannot write the sandbox's seccomp filter".into()))?;
    let seccomp = std::fs::File::open(&filter).or_raise(|| CliError("cannot open the sandbox's seccomp filter".into()))?;
    sandbox::inherit(&seccomp).or_raise(|| CliError("cannot hand bubblewrap the seccomp filter".into()))?;
    // Relays run first inside, then the command.
    let mut inside: Vec<OsString> = Vec::new();
    if !plan.relays.is_empty() {
        let exe = std::env::current_exe().or_raise(|| CliError("cannot find the silt executable".into()))?;
        inside.extend([exe.into_os_string(), "sandbox-init".into()]);
        for relay in &plan.relays {
            inside.push("--relay".into());
            inside.push(format!("{}={}", relay.port, relay.socket.display()).into());
        }
        inside.push("--".into());
    }
    inside.extend(command.iter().cloned());
    let args = sandbox::bwrap_args(&plan, sandbox::Ids::here(), &seccomp, resolv.as_deref(), &inside);
    let error = std::process::Command::new(&tools.bwrap).args(args).env_remove("SILT_SANDBOX_IDS").exec();
    Err(fail(format!("cannot start bubblewrap ({}): {error}", tools.bwrap.display())))
}

#[cfg(not(target_os = "linux"))]
pub fn run(_plan: &Path, _in_network: bool, _command: &[OsString]) -> Result<(), Exn<CliError>> {
    Err(fail("Silt's sandbox runs on Linux for now; on this system agents use their own sandbox"))
}

/// `silt sandbox-init`: the first process of a sandbox with relays. Each
/// listens on the sandbox's own `127.0.0.1` and passes what arrives to a
/// socket outside it, such as the gateway's; then the command runs, and
/// this exits as it did.
#[cfg(unix)]
pub fn init(relays: &[String], command: &[OsString]) -> Result<(), Exn<CliError>> {
    use std::net::TcpListener;
    use std::os::unix::net::UnixStream;

    use exn::ResultExt as _;

    for relay in relays {
        let (port, socket) = relay.split_once('=').ok_or_else(|| fail(format!("a relay is PORT=SOCKET, not {relay}")))?;
        let listener = TcpListener::bind(("127.0.0.1", port.parse::<u16>().map_err(|_| fail(format!("{port} isn't a port")))?))
            .or_raise(|| CliError(format!("cannot listen on port {port} in the sandbox")))?;
        let socket = std::path::PathBuf::from(socket);
        std::thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let socket = socket.clone();
                std::thread::spawn(move || {
                    let Ok(server) = UnixStream::connect(&socket) else { return };
                    let (Ok(mut from_client), Ok(mut from_server)) = (client.try_clone(), server.try_clone()) else { return };
                    let (mut to_client, mut to_server) = (client, server);
                    let up = std::thread::spawn(move || {
                        let _ = std::io::copy(&mut from_client, &mut to_server);
                        let _ = to_server.shutdown(std::net::Shutdown::Write);
                    });
                    let _ = std::io::copy(&mut from_server, &mut to_client);
                    let _ = to_client.shutdown(std::net::Shutdown::Write);
                    let _ = up.join();
                });
            }
        });
    }
    let (program, args) = command.split_first().ok_or_else(|| fail("no command to run"))?;
    let status = std::process::Command::new(program)
        .args(args)
        .status()
        .or_raise(|| CliError(format!("cannot start {}", program.to_string_lossy())))?;
    if let Some(signal) = std::os::unix::process::ExitStatusExt::signal(&status) {
        std::process::exit(128 + signal);
    }
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(not(unix))]
pub fn init(_relays: &[String], _command: &[OsString]) -> Result<(), Exn<CliError>> {
    Err(fail("Silt's sandbox runs on Linux for now"))
}
