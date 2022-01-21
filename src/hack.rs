use guppy::graph::feature::FeatureId;
use guppy::graph::{feature::StandardFeatures, DependencyDirection, PackageGraph};
use guppy::platform::{Platform, PlatformStatus};
use guppy::{DependencyKind, PackageId};
use petgraph::adj::NodeIndex;
use petgraph::algo::toposort;
use petgraph::algo::tred::{dag_to_toposorted_adjacency_list, dag_transitive_reduction_closure};
use petgraph::visit::{EdgeRef, IntoNeighborsDirected, IntoNodeIdentifiers, Visitable};
use petgraph::{EdgeDirection, Graph};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use tracing::{debug, info, trace, trace_span, warn};

use crate::{query::*, toml};

type Changeset<'a> = BTreeMap<&'a PackageId, BTreeMap<&'a PackageId, BTreeSet<&'a str>>>;

pub fn check(package_graph: &PackageGraph) -> anyhow::Result<()> {
    for package in package_graph
        .resolve_workspace()
        .packages(DependencyDirection::Forward)
    {
        let meta = package.metadata_table();
        if !&meta["hackerman"]["lock"].is_null() {
            debug!("Verifying checksum in {}", package.manifest_path());
            toml::verify_checksum(package.manifest_path())?;
        }
    }

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
        anyhow::bail!("Changes are required");
    }
    Ok(())
}

/// if a imports b directly or indirectly
///
fn ws_depends_on(
    package_graph: &PackageGraph,
    a: &PackageId,
    b: &PackageId,
    kind: DependencyKind,
) -> anyhow::Result<bool> {
    if a == b {
        return Ok(false);
    }
    Ok(package_graph
        .query_forward([a])?
        .resolve_with(Walker(Place::Workspace))
        .contains(b)?)
}

fn get_changeset(package_graph: &PackageGraph) -> anyhow::Result<Changeset> {
    let kind = DependencyKind::Normal;

    let feature_graph = package_graph.feature_graph();

    let workspace_set = feature_graph
        .query_workspace(StandardFeatures::Default)
        .resolve_with(Walker(Place::Both));

    let mut needs_fixing = BTreeMap::new();

    // for every workspace member separately
    for member in package_graph.workspace().iter() {
        trace_span!("first pass", member = member.name());
        // we iterate over all their direct and transitive dependencies
        // of a given kind, ignoring macro dependen
        for dep in feature_graph
            .query_directed([member.default_feature_id()], DependencyDirection::Forward)?
            .resolve_with(Walker(Place::Both))
            .packages_with_features(DependencyDirection::Forward)
        {
            // dependency comes with a different set of features - it needs to be fixed
            if workspace_set.features_for(dep.package().id())?.as_ref() != Some(&dep) {
                needs_fixing.entry(dep.package().id()).or_insert_with(|| {
                    trace!(
                        name = %dep.package().name(),
                        version = %dep.package().version(),
                        "needs fixing"
                    );
                    dep
                });
            }
        }
    }
    if needs_fixing.is_empty() {
        info!("Nothing to do");
        return Ok(Changeset::default());
    }
    info!("{} package(s) needs fixing", needs_fixing.len());

    let mut patches_to_add: Changeset = BTreeMap::new();

    // next we going over all the dependencies to add and trying to add them at the intersection
    // points with the workspace
    for (dep, _) in needs_fixing.into_iter() {
        let dep_features_in_workspace = workspace_set
            .features_for(dep)?
            .unwrap()
            .features()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();

        for entry_point in package_graph
            .query_reverse([dep])?
            .resolve_with(Walker(Place::External))
            .filter(DependencyDirection::Forward, |p| p.in_workspace())
            .packages(DependencyDirection::Forward)
        {
            // take all the features enabled in the workspace
            let mut vals = dep_features_in_workspace.clone();

            debug!("Checking {} imported by {}", dep, entry_point.name());
            debug!("workspace features:   {:?}", vals);

            // subtract all the features already available at that point
            for present in feature_graph
                .query_forward([entry_point.default_feature_id()])?
                .resolve_with(Walker(Place::Both))
                .features_for(dep)?
                .iter()
                .flat_map(|x| x.features())
            {
                vals.remove(present);
            }

            debug!("missing features:     {:?}", vals);

            // if all the dependencies are present already we can skip it
            if vals.is_empty() {
                trace!("All features are present, skipping");
                continue;
            }

            // otherwise we can skip anything that's enabled by default if "default" is requested
            if vals.contains("default") {
                for def_feat in feature_graph
                    .query_forward([package_graph.metadata(dep)?.default_feature_id()])?
                    .resolve()
                    .packages_with_features(DependencyDirection::Forward)
                    .next()
                    .unwrap()
                    .into_features()
                {
                    vals.remove(def_feat);
                }
            }

            // and include anything requested by the entry point itself which might include default
            if kind == DependencyKind::Development {
                for link in entry_point.direct_links() {
                    if link.to().id() == dep
                        && link.req_for_kind(DependencyKind::Normal).is_present()
                        || link.req_for_kind(DependencyKind::Development).is_present()
                    {
                        for feat in link.req_for_kind(DependencyKind::Normal).features() {
                            vals.insert(feat);
                        }
                        for feat in link.req_for_kind(DependencyKind::Development).features() {
                            vals.insert(feat);
                        }
                    }
                }
            } else {
                for link in entry_point.direct_links() {
                    if link.to().id() == dep && link.req_for_kind(kind).is_present() {
                        for feat in link.req_for_kind(kind).features() {
                            vals.insert(feat);
                        }
                    }
                }
            }

            if dep_features_in_workspace.contains("default") {
                vals.insert("default");
            }

            debug!("Non default features: {:?}", vals);

            patches_to_add
                .entry(entry_point.id())
                .or_insert_with(BTreeMap::new)
                .insert(dep, vals);
        }
    }
    info!(
        "Need to patch {} Cargo.toml file(s) (before trimming)",
        patches_to_add.len()
    );

    // and the last iteration is going across the workspace removing features unified by children
    for member in package_graph
        .query_workspace()
        .resolve_with(Walker(Place::Both))
        .packages(DependencyDirection::Reverse)
    {
        if !member.in_workspace() {
            continue;
        }

        // that is if a member defines any patches
        if let Some(child_patch) = patches_to_add.get(member.id()).cloned() {
            trace!("Checking features forced by {}", member.name());
            for (&patch_id, patch) in patches_to_add.iter_mut() {
                // we look for all the packages that import it
                //
                // TODO: depends_on is slow with all the nested loops
                if ws_depends_on(package_graph, patch_id, member.id(), kind)? {
                    patch.retain(|dep, feats| child_patch.get(dep) != Some(feats));
                }
            }
        }
    }

    // insert implicit "default" feature
    for member_patch in patches_to_add.values_mut() {
        for (patched, changes) in member_patch.iter_mut() {
            if feature_graph
                .metadata(FeatureId::new(patched, "default"))
                .is_err()
            {
                changes.insert("default");
            }
        }
    }

    info!(
        "Need to patch {} Cargo.toml file(s) (after trimming)",
        patches_to_add.len()
    );

    Ok(patches_to_add)
}

#[derive(Clone, Hash, Ord, PartialOrd, Eq, PartialEq, Debug)]
enum Pla {
    Always,
    //    Cond,
}

type FeatureMap<'a> = BTreeMap<FeatureId<'a>, BTreeSet<(FeatureId<'a>, Pla)>>;

fn follow(here: &Platform, status: PlatformStatus) -> Option<Pla> {
    match status {
        PlatformStatus::Never => None,
        PlatformStatus::Always => Some(Pla::Always),
        PlatformStatus::PlatformDependent { eval } => match eval.eval(here) {
            guppy::platform::EnabledTernary::Disabled => None,
            guppy::platform::EnabledTernary::Unknown => todo!(),
            guppy::platform::EnabledTernary::Enabled => Some(Pla::Always),
        },
    }
}

// -----------------------------------------------------------------------------------------------

struct FG<'a>(&'a Graph<FeatureId<'a>, Pla>);

impl<'a> dot::GraphWalk<'a, FeatureId<'a>, (FeatureId<'a>, FeatureId<'a>)> for FG<'a> {
    fn nodes(&'a self) -> dot::Nodes<'a, FeatureId<'a>> {
        Cow::from(self.0.node_weights().cloned().collect::<Vec<_>>())
    }

    fn edges(&'a self) -> dot::Edges<'a, (FeatureId<'a>, FeatureId<'a>)> {
        Cow::from(
            self.0
                .edge_references()
                .map(|e| {
                    let src = e.source();
                    let tgt = e.target();
                    (self.0[src], self.0[tgt])
                })
                .collect::<Vec<_>>(),
        )
    }

    fn source(&'a self, edge: &(FeatureId<'a>, FeatureId<'a>)) -> FeatureId<'a> {
        edge.0
    }

    fn target(&'a self, edge: &(FeatureId<'a>, FeatureId<'a>)) -> FeatureId<'a> {
        edge.1
    }
}

impl<'a> dot::Labeller<'a, FeatureId<'a>, (FeatureId<'a>, FeatureId<'a>)> for FG<'a> {
    fn graph_id(&'a self) -> dot::Id<'a> {
        dot::Id::new("features").unwrap()
    }

    fn node_id(&'a self, n: &FeatureId<'a>) -> dot::Id<'a> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        n.to_string().hash(&mut hasher);
        let x = hasher.finish();

        dot::Id::new(format!("n{}", x)).unwrap()
    }

    fn node_label(&'a self, node: &FeatureId<'a>) -> dot::LabelText<'a> {
        let s = node.package_id().to_string();
        let name = s.split(' ').collect::<Vec<_>>();

        let fmt = format!("{}\n{}", name[0], node.feature().unwrap_or("n/a"));

        dot::LabelText::label(fmt)
    }
}
// -----------------------------------------------------------------------------------------------

pub fn apply(package_graph: &PackageGraph, _dry: bool, _lock: bool) -> anyhow::Result<()> {
    let feature_graph = package_graph.feature_graph();

    let here = Platform::current()?;

    let mut graph = Graph::new();
    let mut workspace_features = BTreeSet::new();

    let mut nodes = BTreeMap::new();
    for pkt in package_graph.workspace().iter() {
        workspace_features.insert(pkt.default_feature_id());
        nodes.insert(
            pkt.default_feature_id(),
            graph.add_node(pkt.default_feature_id()),
        );
    }

    feature_graph
        .query_workspace(StandardFeatures::Default)
        .resolve_with_fn(|_query, link| {
            let parent_node = link.from();
            let child_node = link.to();
            let mut to_follow = None;

            // first we try to figure out if we want to follow this link.
            // proc-macro links are always ignored
            // dev links outside of the workspace are ignored
            // otherwise links are followed depending on the Platform
            if !child_node.package().is_proc_macro() {
                to_follow = follow(&here, link.normal());
                if to_follow.is_none() && parent_node.package().in_workspace() {
                    to_follow = follow(&here, link.dev());
                }
            }

            let cond = match to_follow {
                Some(cond) => cond,
                None => return false,
            };

            let mut first_visit = false;
            let default_ix = *nodes.entry(link.to().feature_id()).or_insert_with(|| {
                first_visit = true;
                graph.add_node(link.to().feature_id())
            });

            // on the first visit to a new feature node we also store intra package feature
            // dependencies
            if first_visit {
                for f in feature_graph
                    .query_forward([link.to().feature_id()])
                    .unwrap()
                    .resolve_with_fn(|_query, _link| false)
                    .feature_ids(DependencyDirection::Forward)
                {
                    if f != link.to().feature_id() {
                        let to = *nodes.entry(f).or_insert_with(|| graph.add_node(f));
                        graph.add_edge(default_ix, to, Pla::Always);
                    }
                }
            };

            let from = *nodes
                .entry(link.from().feature_id())
                .or_insert_with(|| graph.add_node(link.from().feature_id()));
            let to = *nodes
                .entry(link.to().feature_id())
                .or_insert_with(|| graph.add_node(link.to().feature_id()));

            if !link.to().feature_id().is_base() {
                let base = FeatureId::base(link.to().package_id());
                nodes.entry(base).or_insert_with(|| {
                    let base_node = graph.add_node(base);
                    graph.add_edge(base_node, from, Pla::Always);
                    base_node
                });
            }

            //assert!(!g.contains_edge(from, to));
            trace!(
                "{:3}: -> {:3}: {}-> {} / {cond:?}",
                from.index(),
                to.index(),
                link.from().feature_id(),
                link.to().feature_id()
            );
            graph.add_edge(from, to, cond);

            true
        });
    /*
    // "base" packages don't matter form feature resolution point if view. If they are
    // imported - all the dependencies are. If they are not imported - none are.
    graph.retain_nodes(|a, n| {
        let fid = a[n];
        !fid.is_base() || workspace_features.contains(&fid)
    });*/

    /*
    graph.retain_edges(|a, n| {
        if let Some((from, _)) = a.edge_endpoints(n) {
            a[from].feature() != Some("default")
        } else {
            false
        }
    });*/
    /*
        // Removing redundant edges with using transitive reduction
        let toposort = toposort(&graph, None).expect("cycling dependencies are not supported");
        let (adj_list, revmap) = dag_to_toposorted_adjacency_list::<_, NodeIndex>(&graph, &toposort);
        let (reduction, _closure) = dag_transitive_reduction_closure(&adj_list);
        let before = graph.edge_count();
        graph.retain_edges(|x, y| {
            if let Some((f, t)) = x.edge_endpoints(y) {
                reduction.contains_edge(revmap[f.index()], revmap[t.index()])
            } else {
                false
            }
        });
        let after = graph.edge_count();
        println!("Transitive reduction {} -> {}", before, after);
    */
    transitive_reduction(&mut graph);

    /*
        loop {
            let mut change = false;

            // removing base nodes sitting at the edge - they are not going to affect the results
            let to_drop = graph
                .externals(EdgeDirection::Outgoing)
                .filter(|i| {
                    let fid = graph[*i];
                    fid.is_base() && !workspace_features.contains(&fid)
                })
                .collect::<Vec<_>>();
            println!("trimming base {}", to_drop.len());
            change |= !to_drop.is_empty();

            for e in to_drop.into_iter() {
                graph.remove_node(e);
            }

            let x = graph
                .externals(EdgeDirection::Outgoing)
                .filter(|i| {
                    let fid = graph[*i];
                    if workspace_features.contains(&fid) {
                        return false;
                    }
                    graph
                        .neighbors_directed(*i, EdgeDirection::Incoming)
                        .count()
                        == 1
                })
                .collect::<Vec<_>>();

            println!("trimming single ends {}", x.len());
            for e in x {
                change = true;
                graph.remove_node(e);
            }
            if !change {
                break;
            }
        }
    */
    //    transitive_reduction(&mut graph);

    #[cfg(feature = "spawn_xdot")]
    {
        use tempfile::NamedTempFile;
        let mut file = NamedTempFile::new()?;
        dot::render(&FG(&graph), &mut file)?;
        std::process::Command::new("xdot")
            .args([file.path()])
            .output()?;
    }

    /*

    let mut gg = graph.clone();

    let mut m: FeatureMap = BTreeMap::new();

    let mut frontline_edges = Vec::new();
    let mut frontline_nodes = Vec::new();
    loop {
        frontline_edges.clear();
        frontline_nodes.clear();

        for external in gg.externals(petgraph::Direction::Outgoing) {
            let feature = gg[external];

            let feature_deps = m.remove(&feature).unwrap_or_default();

            frontline_nodes.push(external);
            //            println!("{feature}");
            for edge in gg.edges_directed(external, petgraph::Direction::Incoming) {
                frontline_edges.push(edge.id());

                let usage = edge.weight();
                let user = gg[edge.source()];

                // TODO: THIS should be transformed with usage!
                let mut victim = feature_deps.clone();
                let set = m.entry(user).or_insert_with(BTreeSet::new);
                set.append(&mut victim);
                set.insert((feature, usage.clone()));

                //                println!("\t{} : {:?}", user, usage);
                //                println!("{:?}, should be {:?}", m[&user], feature_deps);
            }

            //            progress = true;
        }

        println!();
        for (k, vs) in m.iter() {
            println!("{k}");
            for v in vs.iter() {
                println!("\t{v:?}");
            }
        }

        println!(
            "{} edges / {} nodes to remove",
            frontline_edges.len(),
            frontline_nodes.len()
        );
        if frontline_edges.is_empty() {
            break;
        } else {
            for e in frontline_edges.iter() {
                gg.remove_edge(*e);
            }
            for n in frontline_nodes.iter() {
                gg.remove_node(*n);
            }
        }

        println!();
        println!();
    }*/

    Ok(())
}

pub fn apply1(package_graph: &PackageGraph, dry: bool, lock: bool) -> anyhow::Result<()> {
    let kind = DependencyKind::Normal;
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
        return Ok(());
    }

    if map.is_empty() {
        info!("Nothing to do, exiting");
        return Ok(());
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

    for package in package_graph
        .query_workspace()
        .resolve_with(Walker(Place::Workspace))
        .packages(DependencyDirection::Reverse)
    {
        if !package.in_workspace() {
            continue;
        }

        if let Some(patch) = map.get(package.id()) {
            info!("Patching {}", package.id());
            crate::toml::set_dependencies(package.manifest_path(), package_graph, patch, lock)?;
        }
    }

    Ok(())
}

pub fn restore(package_graph: PackageGraph) -> anyhow::Result<()> {
    let kind = DependencyKind::Normal;
    let mut changes = false;
    for package in package_graph
        .query_workspace()
        .resolve_with(Walker(Place::Workspace))
        .packages(DependencyDirection::Forward)
    {
        let hacked = package.metadata_table()["hackerman"]["stash"]
            .as_object()
            .is_some();

        if hacked {
            changes = true;
            info!("Restoring {:?}", package.manifest_path());
            crate::toml::restore_dependencies(package.manifest_path())?;
        }
    }
    if !changes {
        warn!("Nothing to do!");
    }

    Ok(())
}

pub fn restore_file(path: &OsStr) -> anyhow::Result<()> {
    crate::toml::restore_dependencies(path)?;
    Ok(())
}

fn transitive_reduction(graph: &mut Graph<FeatureId, Pla>) {
    let before = graph.edge_count();
    let toposort = toposort(&*graph, None).expect("cycling dependencies are not supported");
    let (adj_list, revmap) = dag_to_toposorted_adjacency_list::<_, NodeIndex>(&*graph, &toposort);
    let (reduction, _closure) = dag_transitive_reduction_closure(&adj_list);

    graph.retain_edges(|x, y| {
        if let Some((f, t)) = x.edge_endpoints(y) {
            reduction.contains_edge(revmap[f.index()], revmap[t.index()])
        } else {
            false
        }
    });
    let after = graph.edge_count();
    debug!("Transitive reduction, edges {before} -> {after}");
}
