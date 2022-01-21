use std::{ffi::OsStr, path::Path};

use tempfile::Builder;

use crate::toml::restore_dependencies;

pub fn merge(base: &OsStr, local: &OsStr, remote: &OsStr, merged: &OsStr) -> anyhow::Result<()> {
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

    let merged_bytes = output.stdout;
    let code = output.status;

    std::fs::write(merged, &merged_bytes)?;

    std::process::exit(code.code().unwrap_or(-1));
}
