use anyhow::Context;
use cargo_hackerman::{
    explain::explain,
    hack::hack,
    mergetool,
    opts::{self, Action},
    toml,
};
use cargo_metadata::camino::Utf8PathBuf;
use cargo_platform::Cfg;
use std::{collections::BTreeSet, str::FromStr};
use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

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

fn get_cfgs() -> anyhow::Result<Vec<Cfg>> {
    let output = std::process::Command::new("rustc")
        .arg("--print=cfg")
        .output()
        .context("rustc failed to run")?;
    let stdout = String::from_utf8(output.stdout).unwrap();
    Ok(stdout
        .lines()
        .map(Cfg::from_str)
        .collect::<Result<Vec<_>, _>>()?)
}

fn main() -> anyhow::Result<()> {
    match opts::action().run() {
        Action::Hack { profile, dry, lock } => {
            start_subscriber(profile.verbosity);
            let metadata = profile.exec()?;
            let platform = target_spec::Platform::current()?;
            let triplets = vec![platform.triple_str()];
            let cfgs = get_cfgs()?;
            hack(dry, lock, &metadata, triplets, cfgs)?;
            // regenerate Cargo.lock file
            profile.exec()?;
        }

        Action::Restore { profile, single } => {
            start_subscriber(profile.verbosity);
            let mut changed = false;
            if let Some(path) = single {
                let utf8_path = Utf8PathBuf::try_from(path)?;
                changed |= toml::restore(&utf8_path)?;
            } else {
                let metadata = profile.exec()?;
                let members = metadata.workspace_members.iter().collect::<BTreeSet<_>>();
                for package in &metadata.packages {
                    if members.contains(&package.id) {
                        changed |= toml::restore(&package.manifest_path)?;
                    }
                }
            }
            if changed {
                // regenerate Cargo.lock file
                profile.exec()?;
            }
        }

        Action::Check { profile } => {
            let metadata = profile.exec()?;
            let members = metadata.workspace_members.iter().collect::<BTreeSet<_>>();
            for package in &metadata.packages {
                if members.contains(&package.id) {
                    toml::verify_checksum(package.manifest_path.as_std_path())?;
                }
            }
            let platform = target_spec::Platform::current()?;
            let triplets = vec![platform.triple_str()];
            let cfgs = get_cfgs()?;
            hack(true, false, &metadata, triplets, cfgs)?;
        }

        Action::MergeDriver {
            base,
            local,
            remote,
            result,
        } => {
            mergetool::merge(&base, &local, &remote, &result)?;
        }

        Action::Explain {
            profile,
            krate,
            feature,
            version,
        } => {
            use cargo_hackerman::feat_graph::FeatGraph;
            let metadata = profile.exec()?;
            let platform = target_spec::Platform::current()?;
            let triplets = vec![platform.triple_str()];
            let cfgs = get_cfgs()?;
            let mut fg = FeatGraph::init(&metadata, triplets, cfgs)?;
            fg.optimize()?;

            explain(&mut fg, &krate)?;
        }
    }
    Ok(())
}
