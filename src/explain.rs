use crate::{
    feat_graph::{FeatGraph, HasIndex},
    hack::Collect,
    metadata::{DepKindInfo, Link},
};
use cargo_metadata::Version;
use petgraph::visit::{Dfs, EdgeFiltered, EdgeRef, IntoEdgesDirected, Reversed};
use std::collections::BTreeSet;
use tracing::{debug, info};

pub fn explain<'a>(
    fg: &'a mut FeatGraph<'a>,
    krate: &str,
    feature: Option<&String>,
    version: Option<&Version>,
    package_nodes: bool,
) -> anyhow::Result<()> {
    fg.shrink_to_target()?;
    let mut packages = fg
        .features
        .node_indices()
        .filter(|ix| {
            if let Some(fid) = fg.features[*ix].fid() {
                let package = fid.pid.package();
                if package.name != krate {
                    return false;
                }
                if let Some(feat) = feature {
                    if fid.pid.named(feat) != fid {
                        return false;
                    }
                } else if fid.pid.root() != fid {
                    return false;
                }
                if let Some(ver) = version {
                    if package.version != *ver {
                        return false;
                    }
                }
                true
            } else {
                false
            }
        })
        .collect::<Vec<_>>();

    if package_nodes {
        fg.focus_targets = Some(
            packages
                .iter()
                .map(|ix| {
                    let base = fg.features[*ix].fid().unwrap().base();
                    *fg.fid_cache.get(&base).unwrap()
                })
                .collect::<BTreeSet<_>>(),
        );
    } else {
        fg.focus_targets = Some(packages.iter().copied().collect::<BTreeSet<_>>());
    }

    let first = packages
        .pop()
        .ok_or_else(|| anyhow::anyhow!("{krate} is not in use"))?;

    let g = EdgeFiltered::from_fn(Reversed(&fg.features), |e| {
        !fg.features[e.source()].is_workspace()
            && e.weight().satisfies(
                fg.features[e.source()],
                Collect::Target,
                &fg.platforms,
                &fg.cfgs,
            )
    });

    let mut dfs = Dfs::new(&g, first);

    let mut nodes = BTreeSet::new();
    let mut edges = BTreeSet::new();
    let mut new_edges = BTreeSet::new();

    debug!("Collecting dependencies");
    loop {
        while let Some(node) = dfs.next(&g) {
            if node == fg.root {
                continue;
            }

            let this_node;
            if package_nodes {
                let base = fg.features[node].fid().unwrap().base();
                this_node = *fg.fid_cache.get(&base).unwrap();
                nodes.insert(this_node);
            } else {
                this_node = node;
                nodes.insert(node);
            }
            for edge in g.edges_directed(node, petgraph::EdgeDirection::Outgoing) {
                if edge.target() != fg.root {
                    if package_nodes {
                        let other_node = fg.features[edge.target()].fid().unwrap().base();
                        new_edges.insert((other_node, this_node));
                    } else {
                        edges.insert(edge.id());
                    }
                }
            }
        }
        if let Some(next) = packages.pop() {
            dfs.move_to(next)
        } else {
            break;
        }
    }

    if package_nodes {
        for (a, b) in new_edges {
            let a = a.get_index(fg)?;
            if a == b {
                continue;
            }
            let link = Link {
                optional: false,
                kinds: vec![DepKindInfo::NORMAL],
            };
            edges.insert(fg.features.add_edge(a, b, link));
            //            fg.add_edge(a, b, false, DepKindInfo::NORMAL)?;
        }
    }

    info!("Done traversing");

    fg.focus_nodes = Some(nodes);
    fg.focus_edges = Some(edges);

    #[cfg(feature = "spawn_xdot")]
    {
        let mut file = tempfile::NamedTempFile::new()?;
        dot::render(fg, &mut file)?;
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

/*
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

    let place = if packages.iter().any(|p| p.in_workspace()) {
        Place::Both
    } else {
        Place::External
    };

    let walker = Walker(place);
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

    let fid = FeatureId::new(resolve_package(package_graph, pkg, version)?.id(), feat);
    let place = if package_graph.metadata(fid.package_id())?.in_workspace() {
        Place::Both
    } else {
        Place::External
    };
    let walker = Walker(place);
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
*/
