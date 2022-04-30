use crate::toml::restore;
use cargo_metadata::camino::Utf8PathBuf;
use std::path::Path;

fn restore_path(path: &Path) -> anyhow::Result<()> {
    match path.to_str() {
        Some(d) => restore(&Utf8PathBuf::from(d))?,
        None => crate::toml::restore_path(path)?,
    };
    Ok(())
}

pub fn merge(base: &Path, local: &Path, remote: &Path, _merged: &Path) -> anyhow::Result<()> {
    restore_path(local)?;
    restore_path(base)?;
    restore_path(remote)?;

    let output = std::process::Command::new("git")
        .arg("merge-file")
        .args(["-L", "a/Cargo.toml"])
        .args(["-L", "base/Cargo.toml"])
        .args(["-L", "b/Cargo.toml"])
        .args([local, base, remote])
        .arg("-p")
        .output()?;

    let merged_bytes = output.stdout;
    let code = output.status;

    std::fs::write(local, &merged_bytes)?;

    std::process::exit(code.code().unwrap_or(-1));
}
