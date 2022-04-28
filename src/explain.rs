use crate::{
    feat_graph::{FeatGraph, HasIndex},
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
        .filter(|&ix| {
            if let Some(fid) = fg.features[ix].fid() {
                let package = fid.pid.package();
                // name must match.
                // feature must match if given, otherwise look for base
                // version must match if given
                package.name == krate
                    && feature.map_or(fid.pid.base() == fid, |f| fid.pid.named(f) == fid)
                    && version.map_or(true, |v| package.version == *v)
            } else {
                false
            }
        })
        .collect::<Vec<_>>();

    info!("Found {} matching package(s)", packages.len());

    if package_nodes {
        fg.focus_targets = Some(
            packages
                .iter()
                .flat_map(|&ix| fg.base_node(ix))
                .collect::<BTreeSet<_>>(),
        );
    } else {
        fg.focus_targets = Some(packages.iter().copied().collect::<BTreeSet<_>>());
    }
    let g = EdgeFiltered::from_fn(Reversed(&fg.features), |e| {
        !fg.features[e.source()].is_workspace()
    });

    let mut dfs = Dfs::new(&g, fg.root);

    let mut nodes = BTreeSet::new();
    let mut edges = BTreeSet::new();
    let mut new_edges = BTreeSet::new();

    debug!("Collecting dependencies");
    while let Some(next) = packages.pop() {
        dfs.move_to(next);
        while let Some(node) = dfs.next(&g) {
            let this_node = if package_nodes {
                fg.base_node(node).unwrap()
            } else {
                node
            };
            nodes.insert(this_node);
            for edge in g.edges_directed(node, petgraph::EdgeDirection::Outgoing) {
                if package_nodes {
                    new_edges.insert((fg.base_node(edge.target()).unwrap(), this_node));
                } else {
                    edges.insert(edge.id());
                }
            }
        }
    }

    if package_nodes {
        for (a, b) in new_edges {
            let a = a.get_index(fg)?;
            if a != b {
                let link = Link {
                    optional: false,
                    kinds: vec![DepKindInfo::NORMAL],
                };
                edges.insert(fg.features.add_edge(a, b, link));
            }
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
