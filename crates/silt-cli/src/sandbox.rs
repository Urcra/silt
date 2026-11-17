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
    let args = sandbox::bwrap_args(&plan, sandbox::Ids::here(), &seccomp, resolv.as_deref(), command);
    let error = std::process::Command::new(&tools.bwrap).args(args).env_remove("SILT_SANDBOX_IDS").exec();
    Err(fail(format!("cannot start bubblewrap ({}): {error}", tools.bwrap.display())))
}

#[cfg(not(target_os = "linux"))]
pub fn run(_plan: &Path, _in_network: bool, _command: &[OsString]) -> Result<(), Exn<CliError>> {
    Err(fail("Silt's sandbox runs on Linux for now; on this system agents use their own sandbox"))
}
