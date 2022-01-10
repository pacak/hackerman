use crate::{resolve_feature, NormalOnly};
use guppy::graph::{DependencyDirection, PackageGraph};
use guppy::platform::PlatformStatus;
use guppy::PackageId;
use std::collections::{HashMap, HashSet};

use tracing::trace;

use crate::resolve_package;

pub fn package(
    package_graph: &PackageGraph,
    pkg: &str,
    version: Option<&str>,
) -> anyhow::Result<()> {
    let pid = resolve_package(package_graph, pkg, version)?;
    let mut to_check = vec![pid];
    let mut queued = HashSet::new();
    let mut causes: HashMap<&PackageId, (&PackageId, PlatformStatus)> = HashMap::new();
    let mut ws_causes: HashSet<&PackageId> = HashSet::new();
    loop {
        let next = match to_check.pop() {
            Some(package_id) => package_graph.metadata(package_id)?,
            None => {
                let mut ws_causes = ws_causes.iter().collect::<Vec<_>>();
                ws_causes.sort();

                if ws_causes.is_empty() {
                    anyhow::bail!("It's a mystery why {:?} is imported", pkg);
                }
                println!("{:?} is a dependency due to", pkg);

                for package_id in ws_causes.iter() {
                    let p = package_graph.metadata(package_id)?;
                    print!("{}", p.name());

                    let mut pid = p.id();
                    while let Some((n, s)) = causes.get(pid) {
                        let p = package_graph.metadata(n)?;
                        if s.is_always() {
                            print!(" -?> {} {}", p.name(), p.version());
                        } else {
                            print!(" -> {} {}", p.name(), p.version());
                        }
                        pid = n;
                    }
                    println!();
                }
                return Ok(());
            }
        };

        if next.in_workspace() {
            ws_causes.insert(next.id());
            continue;
        }

        for link in next.direct_links_directed(DependencyDirection::Reverse) {
            let id = link.from().id();
            if queued.contains(id) {
                continue;
            }

            to_check.push(id);
            queued.insert(id);
            let stat = link.normal().status().optional_status();
            let to = link.to().id();
            causes.insert(link.from().id(), (to, stat));

            trace!(?id, cause = ?next.id(), "Queuing new import to check");
        }
    }
}

pub fn feature(
    package_graph: &PackageGraph,
    pkg: &str,
    version: Option<&str>,
    feat: &str,
) -> anyhow::Result<()> {
    let feature_graph = package_graph.feature_graph();

    //    let fid = resolve_feature(&package_graph, "serde_json", None, "float_roundtrip")?;
    let fid = resolve_feature(package_graph, pkg, version, feat)?;
    if feature_graph.is_default_feature(fid)? {
        println!("Feature {:?} is enabled in {:?} by default", feat, pkg);
        return Ok(());
    }

    let mut to_check = vec![fid.package_id()];
    let mut queued = HashSet::new();
    let mut causes = HashSet::new();
    loop {
        let next = match to_check.pop() {
            Some(x) => package_graph.metadata(x)?,
            None => {
                if causes.is_empty() {
                    anyhow::bail!("It's a mystery why {:?} on {:?} is required", feat, pkg);
                }
                println!("Feature {:?} on {:?} is requested by", feat, pkg);
                for package_id in causes.iter() {
                    let package = package_graph.metadata(package_id)?;
                    print!("- {} {} ", package.name(), package.version());
                    if package.in_workspace() {
                        println!("which is a workspace package ({})", package.manifest_path());
                    } else {
                        println!();
                    }
                }
                return Ok(());
            }
        };
        let feature_subgraph = feature_graph
            .query_forward([next.default_feature_id()])?
            .resolve_with(NormalOnly);

        if feature_subgraph.contains(fid)? {
            trace!(cause = next.name(), "Adding a new temporary cause");
            causes.retain(|prev| !package_graph.depends_on(prev, next.id()).unwrap());

            if !causes
                .iter()
                .any(|prev| package_graph.depends_on(next.id(), prev).unwrap())
            {
                causes.insert(next.id().clone());
            }
        }

        for link in next.direct_links_directed(DependencyDirection::Reverse) {
            let id = link.from().id();
            if !queued.contains(id) {
                to_check.push(id);
                queued.insert(id);
                trace!(
                    child = link.to().name(),
                    parent = link.from().name(),
                    "Queuing new import to check"
                );
            }
        }
    }
}
