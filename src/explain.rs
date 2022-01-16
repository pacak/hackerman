use crate::query::{packages_by_name_and_version, Place, Walker};
use crate::resolve_package;
use guppy::graph::feature::{FeatureGraph, FeatureId};
use guppy::graph::{DependencyDirection, PackageGraph};
use guppy::DependencyKind;
use std::collections::{BTreeMap, BTreeSet};

pub fn package(
    package_graph: &PackageGraph,
    name: &str,
    version: Option<&str>,
    kind: DependencyKind,
) -> anyhow::Result<()> {
    let packages = packages_by_name_and_version(package_graph, name, version)?;

    let f = packages.iter().map(|p| FeatureId::base(p.id())).collect();
    let feature_graph = package_graph.feature_graph();
    let walker = Walker(kind, Place::External);
    feature_ids(&feature_graph, f, walker, DependencyDirection::Reverse)
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
    let walker = Walker(kind, Place::External);
    feature_ids(
        &feature_graph,
        vec![fid],
        walker,
        DependencyDirection::Reverse,
    )
}

/// Follow from given features with a walker and plot it as DOT dep graph
pub fn feature_ids(
    feature_graph: &FeatureGraph,
    fid: Vec<FeatureId>,
    walker: Walker,
    dir: DependencyDirection,
) -> anyhow::Result<()> {
    let roots = fid.iter().map(|f| f.package_id()).collect::<BTreeSet<_>>();
    let set = feature_graph.query_directed(fid, dir)?.resolve_with(walker);
    //    let set = feature_graph.query_reverse(fid)?.resolve_with(walker);

    let mut nodes = BTreeMap::new();
    let mut edges = BTreeMap::new();
    let mut features = BTreeMap::new();

    for &root in &roots {
        nodes.insert(root, feature_graph.package_graph().metadata(root)?);
    }

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

    #[cfg(feature = "spawn_xdot")]
    {
        use tempfile::NamedTempFile;
        let mut file = NamedTempFile::new()?;
        dot::render(&graph, &mut file)?;
        std::process::Command::new("xdot")
            .args([file.path()])
            .output()?;
    }

    #[cfg(not(feature = "spawn_xdot"))]
    {
        dot::render(&graph, &mut std::io::stdout())?;
    }

    Ok(())
}
