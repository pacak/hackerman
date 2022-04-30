use anyhow::Context;
use cargo_hackerman::{
    explain::{explain, tree},
    feat_graph::{FeatGraph, Feature},
    hack::hack,
    mergetool,
    opts::{self, Action},
    toml,
};
use cargo_metadata::camino::Utf8PathBuf;
use cargo_platform::Cfg;
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};
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
        Action::Hack {
            profile,
            dry,
            lock,
            no_dev,
        } => {
            start_subscriber(profile.verbosity);
            let metadata = profile.exec()?;
            let platform = target_spec::Platform::current()?;
            let triplets = vec![platform.triple_str()];
            let cfgs = get_cfgs()?;
            hack(dry, lock, no_dev, &metadata, triplets, cfgs)?;
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

        Action::Check { profile, no_dev } => {
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
            hack(true, false, no_dev, &metadata, triplets, cfgs)?;
        }

        Action::MergeDriver {
            base,
            local,
            remote,
            result,
        } => {
            mergetool::merge(&base, &local, &remote, &result)?;
        }
        Action::Tree {
            profile,
            no_transitive_opt,
            package_nodes,
            workspace,
            krate,
            feature,
            version,
            no_dev,
        } => {
            start_subscriber(profile.verbosity);
            let metadata = profile.exec()?;
            let platform = target_spec::Platform::current()?;
            let triplets = vec![platform.triple_str()];
            let cfgs = get_cfgs()?;
            let mut fg = FeatGraph::init(&metadata, triplets, cfgs)?;
            fg.optimize(no_transitive_opt)?;
            tree(
                &mut fg,
                krate.as_ref(),
                feature.as_ref(),
                version.as_ref(),
                package_nodes,
                workspace,
                no_dev,
            )?;
        }

        Action::Explain {
            profile,
            krate,
            feature,
            version,
            no_transitive_opt,
            package_nodes,
        } => {
            start_subscriber(profile.verbosity);
            let metadata = profile.exec()?;
            let platform = target_spec::Platform::current()?;
            let triplets = vec![platform.triple_str()];
            let cfgs = get_cfgs()?;
            let mut fg = FeatGraph::init(&metadata, triplets, cfgs)?;
            fg.optimize(no_transitive_opt)?;

            explain(
                &mut fg,
                &krate,
                feature.as_ref(),
                version.as_ref(),
                package_nodes,
            )?;
        }
        Action::ShowCrate {
            profile,
            krate,
            version,
            focus,
        } => {
            let metadata = profile.exec()?;
            let version = version.map(|v| v.to_string());
            let package = metadata
                .packages
                .iter()
                .find(|p| {
                    p.name == krate
                        && version
                            .as_ref()
                            .map_or(true, |v| &p.version.to_string() == v)
                })
                .ok_or_else(|| anyhow::anyhow!("{krate} {version:?} is not used"))?;

            match focus {
                opts::Focus::Manifest => {
                    let path = &package.manifest_path;
                    let orig = path.with_extension("toml.orig");
                    let manifest = if orig.exists() {
                        std::fs::read_to_string(&orig)?
                    } else {
                        std::fs::read_to_string(&path)?
                    };
                    println!("{manifest}");
                    return Ok(());
                }
                opts::Focus::Readme => {
                    if let Some(readme) = &package.readme {
                        println!("{}", std::fs::read_to_string(&readme)?);
                    } else {
                        anyhow::bail!("Package {krate} v{} defines no readme", package.version);
                    }
                }
                opts::Focus::Documentation => {
                    use std::process::Command;
                    // intentionally ignoring documentation field to avoid serde shenanigans
                    let url = format!(
                        "https://docs.rs/{}/{}/{}",
                        package.name, package.version, package.name
                    );

                    if cfg!(target_os = "linux") {
                        Command::new("xdg-open").arg(url).output()?;
                    } else if cfg!(target_os = "windows") {
                        Command::new("start").arg(url).output()?;
                    } else {
                        todo!("How do you open {url} on this OS?");
                    }
                    return Ok(());
                }
            }
        }
        Action::Dupes { profile } => {
            let mut any = false;
            let metadata = profile.exec()?;
            let platform = target_spec::Platform::current()?;
            let triplets = vec![platform.triple_str()];
            let cfgs = get_cfgs()?;
            let mut fg = FeatGraph::init(&metadata, triplets, cfgs)?;
            fg.shrink_to_target()?;

            let mut packages = BTreeMap::new();
            for fid in fg.features.node_weights().filter_map(Feature::fid) {
                if fid == fid.base() {
                    let p = fid.pid.package();
                    packages
                        .entry(p.name.clone())
                        .or_insert_with(Vec::new)
                        .push(p.clone());
                }
            }
            for (name, copies) in &packages {
                if copies.len() < 2 {
                    continue;
                }
                any = true;
                print!("{name}:");
                for c in copies {
                    print!(" {}", c.version);
                }
                println!();
            }
            if !any {
                println!("All packages are present in one version only");
            }
        }
    }
    Ok(())
}
