use guppy::{
    graph::{feature::StandardFeatures, DependencyDirection, PackageGraph},
    PackageId,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use tracing::{debug, info, trace};

use crate::NormalOnly;

/// locate intersection points between `package_id` and  workspace
fn intersect_workspace<'a>(
    pg: &'a PackageGraph,
    package_id: &'a PackageId,
) -> anyhow::Result<HashSet<&'a PackageId>> {
    let mut res = HashSet::new();
    let mut to_check = vec![package_id];
    let mut checking = HashSet::new();
    loop {
        if let Some(package_id) = to_check.pop() {
            let package = pg.metadata(package_id)?;
            for link in package.reverse_direct_links() {
                // only care about normal dependencies for now
                if !link.normal().is_present() {
                    continue;
                }
                let parent = link.from();
                if parent.in_workspace() {
                    res.insert(parent.id());
                } else if !checking.contains(parent.id()) {
                    trace!("checking {:?} via {:?}", parent.name(), package.name());
                    to_check.push(parent.id());
                    checking.insert(parent.id());
                }
            }
        } else {
            return Ok(res);
        }
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub enum Apply {
    Retry,
    Success,
}

type Changeset<'a> = BTreeMap<&'a PackageId, BTreeMap<&'a PackageId, Vec<&'a str>>>;

pub fn check(package_graph: &PackageGraph) -> anyhow::Result<()> {
    let map = get_changeset(package_graph)?;
    if map.is_empty() {
        println!("No changes required")
    } else {
        println!("Following changes may be required");
        for (member, changes) in map.iter() {
            println!("{}", member);
            for (dep, feats) in changes.iter() {
                let dep = package_graph.metadata(dep)?;
                println!("\t{} {} {:?}", dep.name(), dep.version(), feats);
            }
        }
    }
    Ok(())
}

fn get_changeset(package_graph: &PackageGraph) -> anyhow::Result<Changeset> {
    let feature_graph = package_graph.feature_graph();

    let workspace_set = feature_graph
        .query_workspace(StandardFeatures::Default)
        .resolve_with(NormalOnly);

    let mut map: Changeset = BTreeMap::new();
    let mut fixed = HashSet::new();

    for member in package_graph.workspace().iter() {
        for dep in feature_graph
            .query_directed([member.default_feature_id()], DependencyDirection::Forward)?
            .resolve_with(NormalOnly)
            .packages_with_features(DependencyDirection::Forward)
        {
            let ws = workspace_set.features_for(dep.package().id())?.unwrap();
            // nothing to do
            if ws == dep {
                continue;
            }

            // skip wrong versions and complain a bit
            if ws.package().version() != dep.package().version() {
                info!(
                    "Skipping {} because versions don't match: workspace: {}, {}: {}",
                    dep.package().name(),
                    ws.package().version(),
                    member.name(),
                    dep.package().version()
                );
            }

            if fixed.contains(dep.package().id()) {
                trace!(
                    "Skipping previously processed {} {}",
                    dep.package().name(),
                    dep.package().version()
                );
                continue;
            } else {
                fixed.insert(dep.package().id());
            }

            debug!(
                "For package {:?} crate {:?} {} workspace adds {:?} to {:?}",
                member.name(),
                dep.package().name(),
                dep.package().version(),
                ws.features()
                    .iter()
                    .filter(|x| !dep.features().contains(x))
                    .collect::<Vec<_>>(),
                dep.features(),
            );

            for entry_point in intersect_workspace(package_graph, dep.package().id())? {
                let ep = package_graph.metadata(entry_point)?;

                // we can skip the entry point if it already depends on `dep` with all features
                if let Some(existing) = feature_graph
                    .query_directed([ep.default_feature_id()], DependencyDirection::Forward)?
                    .resolve_with(NormalOnly)
                    .features_for(dep.package().id())?
                {
                    if existing == ws {
                        continue;
                    }
                }

                trace!(
                    "adding {:?} to {:?} with {:?}",
                    dep.package().name(),
                    ep.name(),
                    ws.features()
                );

                let ws_entry = map.entry(entry_point).or_insert_with(BTreeMap::new);
                ws_entry
                    .entry(dep.package().id())
                    .or_insert_with(|| ws.clone().into_features());
            }
        }
    }
    Ok(map)
}

pub fn apply(package_graph: &PackageGraph, dry: bool) -> anyhow::Result<Apply> {
    let map = get_changeset(package_graph)?;
    if dry {
        if map.is_empty() {
            println!("Features are unified as is")
        } else {
            println!("Following changes may be required");
            for (member, changes) in map.iter() {
                println!("{}", member);
                for (dep, feats) in changes.iter() {
                    let dep = package_graph.metadata(dep)?;
                    println!("\t{} {} {:?}", dep.name(), dep.version(), feats);
                }
            }
        }
        return Ok(Apply::Success);
    }

    if map.is_empty() {
        info!("Nothing to do, exiting");
        return Ok(Apply::Success);
    } else {
        debug!("Following changes may be required");
        for (member, changes) in map.iter() {
            debug!("{}", member);
            for (dep, feats) in changes.iter() {
                let dep = package_graph.metadata(dep)?;
                debug!("\t{} {} {:?}", dep.name(), dep.version(), feats);
            }
        }
    }

    let mut patched: BTreeSet<&PackageId> = BTreeSet::new();
    let mut retry_needed = false;

    for package in package_graph
        .query_workspace()
        .resolve_with(NormalOnly)
        .packages(DependencyDirection::Reverse)
    {
        if !package.in_workspace() {
            continue;
        }

        if patched
            .iter()
            .any(|prev| package_graph.depends_on(package.id(), prev).unwrap())
        {
            trace!(
                "Skipping {:?} until next iteration",
                package.manifest_path()
            );
            retry_needed = true;
            continue;
        }

        if let Some(patch) = map.get(package.id()) {
            info!("Patching {}", package.id());
            patched.insert(package.id());
            crate::toml::set_dependencies(package.manifest_path(), package_graph, patch)?;
        }
    }

    if retry_needed {
        info!("{} crates patched this iteration", patched.len());

        for p in patched.iter() {
            debug!("- {:?}", p);
        }
        Ok(Apply::Retry)
    } else {
        Ok(Apply::Success)
    }
}
