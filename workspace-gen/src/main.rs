use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use bpaf::Bpaf;

use workspace_gen::config::WorkspaceConfig;
use workspace_gen::fs::{ensure_output_dir, write_workspace};
use workspace_gen::generator;

#[derive(Bpaf)]
#[bpaf(options)]
struct Cli {
    /// Output directory (default: test_workspaces)
    #[bpaf(short, long, argument("DIR"), fallback("test_workspaces".into()))]
    output: PathBuf,

    /// Dry run - show what would be generated without writing files
    #[bpaf(short, long)]
    dry_run: bool,

    /// Path to the TOML configuration file
    #[bpaf(positional("CONFIG"))]
    config: PathBuf,
}

fn main() -> Result<()> {
    let cli = cli().fallback_to_usage().run();

    let content = fs::read_to_string(&cli.config)
        .with_context(|| format!("Failed to read config file: {}", cli.config.display()))?;

    let config: WorkspaceConfig = toml::from_str::<WorkspaceConfig>(&content)
        .with_context(|| format!("Failed to parse config file: {}", cli.config.display()))?
        .merge_duplicates();

    let workspace = generator::generate(&config, &cli.output);

    if !cli.dry_run {
        let ws_dir = cli.output.join(&config.workspace.name);
        ensure_output_dir(&ws_dir)?;
    }

    write_workspace(&workspace, cli.dry_run)?;

    if !cli.dry_run {
        println!("\nWorkspace generated at: {}", workspace.root.display());
    }

    Ok(())
}
