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
                }
                if fid.pid.root() != fid && fid.pid.base() != fid {
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

    info!("Found {} matching package(s)", packages.len());

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
