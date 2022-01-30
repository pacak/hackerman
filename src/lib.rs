#![doc = include_str!("../README.md")]

use guppy::graph::{DependencyDirection, PackageGraph, PackageMetadata, PackageResolver};
use guppy::DependencyKind;
use opts::Focus;

pub mod dump;
pub mod dupes;
pub mod explain;
pub mod feat_graph;
pub mod hack;
pub mod mergetool;
pub mod opts;
pub mod query;
pub mod toml;
pub mod tree;

fn resolve_package<'a>(
    g: &'a PackageGraph,
    name: &'a str,
    mversion: Option<&str>,
) -> anyhow::Result<PackageMetadata<'a>> {
    let set = g.resolve_package_name(name);

    match set.len() {
        0 => anyhow::bail!("Package {} is not in use", name),
        1 => {
            let pkg = set.packages(DependencyDirection::Forward).next().unwrap();
            if let Some(version) = mversion {
                if version != pkg.version().to_string() {
                    anyhow::bail!(
                        "Version {} for {} was requested but {} was found instead",
                        version,
                        name,
                        pkg.version()
                    );
                }
            }
            Ok(pkg)
        }
        _ => {
            let version = mversion.ok_or_else(|| {
                let versions = set
                    .root_packages(DependencyDirection::Forward)
                    .map(|p| p.version().to_string())
                    .collect::<Vec<_>>();
                anyhow::anyhow!(
                    "There are multiple versions of {} but no version is specified, {:?}",
                    name,
                    versions
                )
            })?;
            for pkg in set.packages(DependencyDirection::Forward) {
                if pkg.version().to_string() == version {
                    return Ok(pkg);
                }
            }

            anyhow::bail!("Package {} {} is not in use", name, version);
        }
    }
}

fn dump_file<P>(path: P) -> anyhow::Result<()>
where
    P: AsRef<std::path::Path>,
{
    use std::io::prelude::*;
    let mut file = std::fs::File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    print!("{}", contents);
    Ok(())
}

pub fn show_package(
    package_graph: &PackageGraph,
    name: &str,
    version: Option<&str>,
    focus: Option<Focus>,
) -> anyhow::Result<()> {
    let package = resolve_package(package_graph, name, version)?;

    match focus {
        None => {}
        Some(Focus::Documentation) => {
            // intentionally ignoring documentation field to get the right documentation page for
            // this version
            let url = format!(
                "https://docs.rs/{}/{}/{}",
                package.name(),
                package.version(),
                package.name()
            );

            use std::process::*;
            if cfg!(target_os = "linux") {
                Command::new("xdg-open").arg(url).output()?;
            } else if cfg!(target_os = "windows") {
                Command::new("start").arg(url).output()?;
            } else {
                todo!("How do you open {url} on this OS?");
            }
            return Ok(());
        }
        Some(Focus::Manifest) => {
            let path = package.manifest_path();
            let orig = path.with_extension("toml.orig");
            if orig.exists() {
                dump_file(orig)?;
            } else {
                dump_file(path)?;
            }
            return Ok(());
        }
        Some(Focus::Readme) => {
            if let Some(doc) = package.documentation() {
                dump_file(doc)?;
            } else {
                anyhow::bail!(
                    "Crate {} v{} defies no documentation",
                    package.name(),
                    package.version()
                );
            }
            return Ok(());
        }
    }

    println!(
        "Package:          {} v{}",
        package.name(),
        package.version()
    );
    println!("Authors:          {}", package.authors().join(", "));

    if let Some(home) = package.homepage() {
        println!("Homepage:         {home}")
    }
    if let Some(repo) = package.repository() {
        println!("Repository:       {repo}")
    }
    println!(
        "crates.io:        https://crates.io/crates/{}/{}",
        package.name(),
        package.version()
    );
    if let Some(doc) = package.documentation() {
        println!("Documentation:    {doc}")
    }
    if let Some(readme) = package.readme() {
        let dir = package
            .manifest_path()
            .parent()
            .expect("Manifest can't be root");
        println!("Readme:           {dir}/{readme}")
    }
    println!("Manifest:         {}", package.manifest_path());
    if let Some(descr) = package.description() {
        println!("Description:      {descr}")
    }

    Ok(())
}
