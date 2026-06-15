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
use cargo_metadata::cargo_platform::Cfg;
use std::{
    collections::{BTreeMap, BTreeSet},
    process::Command,
    str::FromStr,
};
use tracing::Level;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

fn start_subscriber((_, level): (usize, Level)) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::default().add_directive(level.into()));
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

fn get_platform(
    meta: &cargo_metadata::Metadata,
) -> Result<target_spec::Platform, target_spec::Error> {
    use std::borrow::Cow;
    use target_spec::*;

    fn read_custom(meta: &cargo_metadata::Metadata) -> Option<Cow<'static, str>> {
        let hm = meta.workspace_metadata.get("hackerman")?.as_object()?;
        hm.get("platform")?
            .as_str()
            .map(|s| Cow::Owned(s.to_owned()))
    }
    fn read_features(meta: &cargo_metadata::Metadata) -> Option<BTreeSet<Cow<'static, str>>> {
        let hm = meta.workspace_metadata.get("hackerman")?.as_object()?;

        let zzz = hm
            .get("features")?
            .as_array()?
            .iter()
            .filter_map(|s| s.as_str().map(|s| Cow::Owned(s.to_owned())))
            .collect();

        Some(zzz)
    }

    let Some(custom) = read_custom(meta) else {
        return Platform::build_target();
    };
    let feats = read_features(meta).unwrap_or_default();
    let feats = target_spec::TargetFeatures::Features(feats);
    Platform::new(custom, feats)
}

fn main() -> anyhow::Result<()> {
    let action = opts::action().fallback_to_usage().run();

    match action {
        Action::Hack { profile, dry, lock } => {
            start_subscriber(profile.verbosity);
            let metadata = profile.exec()?;
            let platform = get_platform(&metadata)?;
            let triplets = vec![platform.triple_str()];
            let cfgs = get_cfgs()?;
            hack(dry, lock, &metadata, triplets, cfgs)?;

            // regenerate Cargo.lock file
            if !dry {
                profile.exec()?;
            }
        }

        Action::Restore { profile, separate } => {
            start_subscriber(profile.verbosity);
            let mut changed = false;
            if separate.is_empty() {
                let metadata = profile.exec()?;
                let members = metadata.workspace_members.iter().collect::<BTreeSet<_>>();
                for package in &metadata.packages {
                    if members.contains(&package.id) {
                        changed |= toml::restore(&package.manifest_path)?;
                    }
                }
            } else {
                for path in separate {
                    let utf8_path = Utf8PathBuf::try_from(path)?;
                    changed |= toml::restore(&utf8_path)?;
                }
            }
            if changed {
                // regenerate Cargo.lock file
                profile.exec()?;
            }
        }

        Action::Check { profile } => {
            start_subscriber(profile.verbosity);
            let metadata = profile.exec()?;
            let members = metadata.workspace_members.iter().collect::<BTreeSet<_>>();
            for package in &metadata.packages {
                if members.contains(&package.id) {
                    toml::verify_checksum(package.manifest_path.as_std_path())?;
                }
            }
            let platform = get_platform(&metadata)?;
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
        Action::Tree {
            profile,
            no_transitive_opt,
            package_nodes,
            workspace,
            krate,
            feature,
            version,
            no_dev,
            stdout,
        } => {
            start_subscriber(profile.verbosity);
            let metadata = profile.exec()?;
            let platform = get_platform(&metadata)?;
            let triplets = vec![platform.triple_str()];
            let cfgs = get_cfgs()?;
            let mut fg = FeatGraph::init(&metadata, triplets, cfgs)?;
            fg.optimize(no_transitive_opt)?;
            tree(
                &mut fg,
                krate.as_ref(),
                feature.as_deref(),
                version.as_ref(),
                package_nodes,
                workspace,
                no_dev,
                stdout,
            )?;
        }

        Action::Explain {
            profile,
            krate,
            feature,
            version,
            no_transitive_opt,
            package_nodes,
            stdout,
        } => {
            start_subscriber(profile.verbosity);
            let metadata = profile.exec()?;
            let platform = get_platform(&metadata)?;
            let triplets = vec![platform.triple_str()];
            let cfgs = get_cfgs()?;
            let mut fg = FeatGraph::init(&metadata, triplets, cfgs)?;
            fg.optimize(no_transitive_opt)?;

            explain(
                &mut fg,
                &krate,
                feature.as_deref(),
                version.as_ref(),
                package_nodes,
                stdout,
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
                    p.name == krate && version.as_ref().is_none_or(|v| &p.version.to_string() == v)
                })
                .ok_or_else(|| anyhow::anyhow!("{krate} {version:?} is not used"))?;

            match focus {
                opts::Focus::Manifest => {
                    let path = &package.manifest_path;
                    let orig = path.with_extension("toml.orig");
                    let manifest = if orig.exists() {
                        std::fs::read_to_string(&orig)?
                    } else {
                        std::fs::read_to_string(path)?
                    };
                    println!("{manifest}");
                    return Ok(());
                }
                opts::Focus::Readme => {
                    let manifest = &package.manifest_path;
                    if let Some(readme) = &package.readme {
                        let readme = manifest.with_file_name(readme);
                        println!("{}", std::fs::read_to_string(readme)?);
                    } else {
                        anyhow::bail!("Package {krate} v{} defines no readme", package.version);
                    }
                }
                opts::Focus::Documentation => {
                    // intentionally ignoring documentation field to avoid serde shenanigans
                    let url = format!("https://docs.rs/{}/{}", package.name, package.version);

                    open_url(&url)?;

                    return Ok(());
                }
                opts::Focus::Repository => {
                    if let Some(url) = &package.repository {
                        open_url(url.as_ref())?;
                    } else {
                        anyhow::bail!("Package {krate} v{} defines no repository", package.version);
                    }
                }
            }
        }
        Action::Dupes { profile } => {
            let mut any = false;
            let metadata = profile.exec()?;
            let platform = get_platform(&metadata)?;
            let triplets = vec![platform.triple_str()];
            let cfgs = get_cfgs()?;
            let mut fg = FeatGraph::init(&metadata, triplets, cfgs)?;
            fg.shrink_to_target()?;

            let mut packages = BTreeMap::new();
            for fid in fg.features.node_weights().filter_map(Feature::fid) {
                if fid == fid.get_base() {
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

fn open_url(url: &str) -> anyhow::Result<()> {
    if cfg!(target_os = "linux") {
        Command::new("xdg-open").arg(url).output()?;
    } else if cfg!(target_os = "windows") {
        Command::new("start").arg(url).output()?;
    } else {
        #[cfg(feature = "webbrowser")]
        {
            webbrowser::open(url)?;
            return Ok(());
        }
        println!("{url}");
    }
    Ok(())
}
