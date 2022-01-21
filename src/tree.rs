use crate::explain::feature_ids;
use crate::query::{packages_by_name_and_version, Place, Walker};
use guppy::graph::feature::StandardFeatures;
use guppy::{graph::PackageGraph, DependencyKind};
use std::collections::BTreeSet;

pub fn workspace(package_graph: &PackageGraph, kind: DependencyKind) -> anyhow::Result<()> {
    let fids = package_graph
        .workspace()
        .iter()
        .map(|p| p.default_feature_id())
        .collect::<Vec<_>>();
    let fg = package_graph.feature_graph();
    let walker = Walker(Place::Workspace);
    feature_ids(
        &fg,
        fids,
        walker,
        guppy::graph::DependencyDirection::Forward,
    )?;

    Ok(())
}

pub fn package(
    package_graph: &PackageGraph,
    name: &str,
    feat: Option<&str>,
    version: Option<&str>,
    kind: DependencyKind,
) -> anyhow::Result<()> {
    let packages = packages_by_name_and_version(package_graph, name, version)?;
    let pids = packages.iter().map(|p| p.id()).collect::<BTreeSet<_>>();
    let fg = package_graph.feature_graph();
    let walker = Walker(Place::Both);

    let fids = fg
        .query_workspace(StandardFeatures::Default)
        .resolve_with(walker)
        .features(guppy::graph::DependencyDirection::Forward)
        .filter(|f| feat.map_or(true, |wanted| f.feature_id().feature() == Some(wanted)))
        .filter_map(|f| pids.contains(f.package_id()).then(|| f.feature_id()))
        .collect::<Vec<_>>();

    feature_ids(
        &fg,
        fids,
        walker,
        guppy::graph::DependencyDirection::Forward,
    )?;
    Ok(())
}
