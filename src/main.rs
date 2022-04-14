use std::ffi::OsStr;

use cargo_hackerman::{
    hack::hack,
    opts::{self, Action},
};
use cargo_metadata::MetadataCommand;
//use guppy::DependencyKind;
use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/*
fn guppy_graph(path: &OsStr) -> anyhow::Result<guppy::graph::PackageGraph> {
    use guppy::{graph::PackageGraph, MetadataCommand};
    let mut cmd = MetadataCommand::new();
    Ok(PackageGraph::from_command(
        cmd.manifest_path(path), // .other_options(["--filter-platform", "x86_64-unknown-linux-gnu"]),
    )?)
}*/

fn start_subscriber(level: Level) {
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
}

fn main() -> anyhow::Result<()> {
    match opts::action().run() {
        Action::Hack { profile, dry, lock } => {
            start_subscriber(profile.verbosity);
            let metadata = profile.exec()?;
            let platform = target_spec::Platform::current()?;
            let triplets = vec![platform.triple_str()];
            hack(dry, lock, &metadata, triplets)?;
        }
    }
    Ok(())

    /*

        println!("{:?}", cargo_hackerman::opts::action().run());

        let (level, manifest, cmd) = cargo_hackerman::opts::options().run();

        let kind = DependencyKind::Normal;
        match cmd {
            Command::Mergedriver(base, local, remote, merged) => {
                mergetool::merge(&base, &local, &remote, &merged)?;
            }
            Command::Explain(e) => {
                let kind = DependencyKind::Normal;

                let g = guppy_graph(&manifest)?;
                match e.feature {
                    Some(feat) => explain::feature(&g, &e.krate, e.version.as_deref(), &feat, kind)?,
                    None => explain::package(&g, &e.krate, e.version.as_deref(), kind)?,
                }
            }
            Command::Hack(Hack { dry, lock }) => {
    <<<<<<< HEAD
                let platform = target_spec::Platform::current()?;

    =======
    >>>>>>> 62f396f
                let mut cmd = cargo_metadata::MetadataCommand::new();
                cmd.manifest_path(&manifest);

                let metadata = cmd.exec().unwrap();

    <<<<<<< HEAD
                let triplets = vec![platform.triple_str()];
                // let triplets = vec![platform.triple_str(), "x86_64-pc-windows-msvc"];
                let mut r = feat_graph::FeatGraph::init(&metadata, triplets)?;

                let (cs, rr) = hack::get_changeset2(&mut r)?;
                hack::apply2(rr, cs)?;
                //            todo!("{cs:?}");
                /*
                let g = guppy_graph(&manifest)?;
                hack::apply(&g, dry, lock)?;*/
    =======
                //hack::hack33(&manifest)?;
                let g = guppy_graph(&manifest)?;
                hack::apply(&g, dry, lock, &metadata)?;
    >>>>>>> 62f396f
            }
            Command::Restore(None) => {
                let g = guppy_graph(&manifest)?;
                hack::restore(g)?;
            }
            Command::Restore(Some(file)) => {
                hack::restore_file(&file)?;
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
            Command::PackageTree(pkg, feat, ver) => {
                let g = guppy_graph(&manifest)?;
                tree::package(&g, &pkg, feat.as_deref(), ver.as_deref(), kind)?;
            }
            Command::ShowPackage(pkg, ver, focus) => {
                let g = guppy_graph(&manifest)?;
                show_package(&g, &pkg, ver.as_deref(), focus)?;
            }
        }
        Ok(())*/
}
