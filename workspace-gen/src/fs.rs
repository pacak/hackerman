use std::fs;
use std::path::Path;

use anyhow::Context;

use crate::generator::GeneratedWorkspace;

pub fn write_workspace(workspace: &GeneratedWorkspace, dry_run: bool) -> anyhow::Result<()> {
    for (path, content) in &workspace.files {
        if dry_run {
            println!("Would create: {}", path.display());
            println!("---");
            println!("{}", content);
            println!("---\n");
        } else {
            let parent = path.parent().context("Failed to get parent directory")?;
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
            fs::write(path, content)
                .with_context(|| format!("Failed to write file: {}", path.display()))?;
            println!("Created: {}", path.display());
        }
    }
    Ok(())
}

pub fn ensure_output_dir(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        // Remove existing workspace directory to start fresh
        fs::remove_dir_all(path)
            .with_context(|| format!("Failed to remove existing directory: {}", path.display()))?;
    }
    Ok(())
}
