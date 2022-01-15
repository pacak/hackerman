use crate::resolve_package;
use guppy::graph::feature::{FeatureGraph, FeatureId};
use guppy::graph::{DependencyDirection, PackageGraph, PackageMetadata};
use guppy::DependencyKind;
use std::collections::{BTreeMap, BTreeSet};

fn packages_by_name_and_version<'a>(
    package_graph: &'a PackageGraph,
    name: &'a str,
    version: Option<&'a str>,
) -> anyhow::Result<Vec<PackageMetadata<'a>>> {
    let mut packages = package_graph
        .resolve_package_name(name)
        .packages(DependencyDirection::Forward)
        .collect::<Vec<_>>();
    let present = !packages.is_empty();
    if let Some(version) = version {
        packages.retain(|p| p.version().to_string() == version);
        if present && packages.is_empty() {
            anyhow::bail!("Package {} v{} is not in use", name, version);
        }
    }
    if packages.is_empty() {
        anyhow::bail!("Package {} is not in use", name)
    }
    Ok(packages)
}

pub fn package(
    package_graph: &PackageGraph,
    name: &str,
    version: Option<&str>,
    kind: DependencyKind,
) -> anyhow::Result<()> {
    let packages = packages_by_name_and_version(package_graph, name, version)?;

    let f = packages.iter().map(|p| FeatureId::base(p.id())).collect();
    let feature_graph = package_graph.feature_graph();
    feature_ids(&feature_graph, f, kind)
}

pub fn feature(
    package_graph: &PackageGraph,
    pkg: &str,
    version: Option<&str>,
    feat: &str,
    kind: DependencyKind,
) -> anyhow::Result<()> {
    let feature_graph = package_graph.feature_graph();

    let fid = FeatureId::new(resolve_package(package_graph, pkg, version)?, feat);
    feature_ids(&feature_graph, vec![fid], kind)
}

/// Follow from given features towards the earliest intersection
/// with the workspace and plot it as DOT dep graph
fn feature_ids(
    feature_graph: &FeatureGraph,
    fid: Vec<FeatureId>,
    kind: DependencyKind,
) -> anyhow::Result<()> {
    let roots = fid.iter().map(|f| f.package_id()).collect::<BTreeSet<_>>();
    let set = feature_graph
        .query_reverse(fid)?
        .resolve_with_fn(|_, link| {
            link.status_for_kind(kind).is_present() && !link.to().package().in_workspace()
        });

    let mut nodes = BTreeMap::new();
    let mut edges = BTreeMap::new();
    let mut features = BTreeMap::new();

    for link in set.cross_links(DependencyDirection::Forward) {
        nodes
            .entry(link.from().package().id())
            .or_insert_with(|| link.from().package());

        nodes
            .entry(link.to().package().id())
            .or_insert_with(|| link.to().package());

        edges
            .entry((link.from().package_id(), link.to().package_id()))
            .or_insert_with(Vec::new)
            .push(link);

        if let Some(feature) = link.to().feature_id().feature() {
            features
                .entry(link.to().package_id())
                .or_insert_with(BTreeSet::new)
                .insert(feature);
        }
    }

    let graph = crate::dump::FeatDepGraph {
        nodes,
        edges,
        features,
        roots,
    };

    dot::render(&graph, &mut std::io::stdout())?;
    Ok(())
}
