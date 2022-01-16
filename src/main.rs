use std::ffi::OsStr;

use cargo_hackerman::{
    dupes, explain, hack,
    opts::{Command, Hack},
    tree,
};
use guppy::DependencyKind;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

fn guppy_graph(path: &OsStr) -> anyhow::Result<guppy::graph::PackageGraph> {
    use guppy::{graph::PackageGraph, MetadataCommand};
    let mut cmd = MetadataCommand::new();
    Ok(PackageGraph::from_command(
        cmd.manifest_path(path)
            .other_options(["--filter-platform", "x86_64-unknown-linux-gnu"]),
    )?)
}

fn main() -> anyhow::Result<()> {
    let (level, manifest, cmd) = cargo_hackerman::opts::options().run();
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| (EnvFilter::default().add_directive(level.into())));
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .without_time()
        .with_level(false);

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .init();

    let kind = DependencyKind::Normal;
    match cmd {
        Command::Explain(e) => {
            let kind = DependencyKind::Normal;

            let g = guppy_graph(&manifest)?;
            match e.feature {
                Some(feat) => explain::feature(&g, &e.krate, e.version.as_deref(), &feat, kind)?,
                None => explain::package(&g, &e.krate, e.version.as_deref(), kind)?,
            }
        }
        Command::Hack(Hack { dry, lock }) => {
            let g = guppy_graph(&manifest)?;
            hack::apply(&g, dry, lock)?;
        }
        Command::Restore(_) => {
            let g = guppy_graph(&manifest)?;
            hack::restore(g)?;
        }
        Command::Verify => {
            let g = guppy_graph(&manifest)?;
            hack::check(&g)?;
        }
        Command::Duplicates => {
            let g = guppy_graph(&manifest)?;
            dupes::list(&g, kind)?;
        }
        Command::WorkspaceTree => {
            let g = guppy_graph(&manifest)?;
            tree::workspace(&g, kind)?;
        }
        Command::PackageTree(pkg, ver) => {
            let g = guppy_graph(&manifest)?;
            tree::package(&g, &pkg, ver.as_deref(), kind)?;
        }
    }
    Ok(())
}
