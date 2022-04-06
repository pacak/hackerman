use std::ffi::OsStr;

use tempfile::Builder;

use crate::toml::restore_dependencies;

pub fn merge(base: &OsStr, local: &OsStr, remote: &OsStr, _merged: &OsStr) -> anyhow::Result<()> {
    let nbase = Builder::new()
        .prefix("Cargo_BASE_")
        .suffix(".toml")
        .rand_bytes(5)
        .tempfile()?;
    std::fs::copy(base, nbase.path())?;
    restore_dependencies(&nbase)?;

    let nlocal = Builder::new()
        .prefix("Cargo_LOCAL_")
        .suffix(".toml")
        .rand_bytes(5)
        .tempfile()?;
    std::fs::copy(local, nlocal.path())?;
    restore_dependencies(&nlocal)?;

    let nremote = Builder::new()
        .prefix("Cargo_REMOTE_")
        .suffix(".toml")
        .rand_bytes(5)
        .tempfile()?;
    std::fs::copy(remote, nremote.path())?;
    restore_dependencies(&nremote)?;

    let output = std::process::Command::new("git")
        .arg("merge-file")
        .args(["-L", "a/Cargo.toml"])
        .args(["-L", "base/Cargo.toml"])
        .args(["-L", "b/Cargo.toml"])
        .args([nlocal.path(), nbase.path(), nremote.path()])
        .arg("-p")
        .output()?;

    // check merge code
    let code = output.status.code().unwrap_or(-1);
    if code != 0 {
        std::process::exit(code);
    }

    let merged_bytes = output.stdout;

    std::fs::write(local, &merged_bytes)?;

    Ok(())
}
