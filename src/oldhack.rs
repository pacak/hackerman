use cargo_metadata::Metadata;
use guppy::graph::feature::FeatureId;
use guppy::graph::{feature::StandardFeatures, DependencyDirection, PackageGraph};
use guppy::platform::{Platform, PlatformStatus};
use guppy::{DependencyKind, PackageId};
use petgraph::algo::toposort;
use petgraph::algo::tred::{dag_to_toposorted_adjacency_list, dag_transitive_reduction_closure};
use petgraph::prelude::NodeIndex;
use petgraph::visit::{Dfs, DfsPostOrder, EdgeRef, NodeFiltered, Walker};
use petgraph::{EdgeDirection, Graph};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use tracing::{debug, info, trace, trace_span, warn};

use crate::feat_graph::{Dep, FeatGraph, FeatGraph2, FeatKind};
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
                if ws_depends_on(package_graph, patch_id, member.id())? {
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

pub fn get_changeset2<'a>(
    package_graph: &'a PackageGraph,
    meta: &'a Metadata,
) -> anyhow::Result<Changeset<'a>> {
    let feature_graph = package_graph.feature_graph();

    let fg2 = FeatGraph2::init(meta)?;

    dump(&fg2);
    //    todo!("{:?}", meta.resolve.as_ref().unwrap().root);

    let mut fg = FeatGraph::init(feature_graph)?;

    let here = Platform::current()?;

    let mut workspace_feats: BTreeMap<&PackageId, BTreeSet<&str>> = BTreeMap::new();

    feature_graph
        .query_workspace(StandardFeatures::Default)
        .resolve_with_fn(|_query, link| {
            // first we try to figure out if we want to follow this link.
            // dev links outside of the workspace are ignored
            // otherwise links are followed depending on the Platform
            let _cond = match follow(&here, link.normal()).or_else(|| follow(&here, link.build())) {
                Some(cond) => cond,
                None => return false,
            };

            let kind = if link.to().package().in_workspace() {
                FeatKind::Workspace
            } else {
                FeatKind::External
            };

            fg.extend_local_feats(link.to().feature_id(), kind).unwrap();

            let from = *fg.nodes.get(&link.from().feature_id()).unwrap();
            let to = fg.feat_index(link.to().feature_id(), kind);
            fg.graph.add_edge(from, to, Dep::Always);
            true
        });

    for feature_id in fg.features.values() {
        if let Some(feat) = feature_id.feature() {
            workspace_feats
                .entry(feature_id.package_id())
                .or_insert_with(BTreeSet::new)
                .insert(feat);
        }
    }

    transitive_reduction(&mut fg.graph);
    let mut changed = BTreeSet::new();

    let workspace_only_graph =
        NodeFiltered::from_fn(&fg.graph, |node| fg.graph[node] != FeatKind::External);

    let members_dfs_postorder = DfsPostOrder::new(&workspace_only_graph, NodeIndex::new(0))
        .iter(&workspace_only_graph)
        .collect::<Vec<_>>();
    for member_ix in members_dfs_postorder {
        if member_ix == NodeIndex::new(0) {
            continue;
        }

        let member = fg.features.get(&member_ix).unwrap();
        println!("Checking {member}");

        let mut deps_feats = BTreeMap::new();

        let mut next = Some(member_ix);
        let mut dfs = Dfs::new(&fg.graph, member_ix);
        let mut made_changes = false;
        'dependency: while let Some(next_item) = next.take() {
            dfs.move_to(next_item);
            while let Some(feat_ix) = dfs.next(&fg.graph) {
                let feat_id = fg.features.get(&feat_ix).unwrap();

                let pkg_id = feat_id.package_id();
                let entry = deps_feats.entry(pkg_id).or_insert_with(BTreeSet::new);

                if let Some(feat) = feat_id.feature() {
                    entry.insert(feat);
                }
            }

            for (dep, feats) in deps_feats.iter() {
                if let Some(ws_feats) = workspace_feats.get(dep) {
                    if ws_feats != feats {
                        if let Some(missing_feat) = ws_feats.difference(feats).next() {
                            println!("\t{missing_feat:?} is missing from {dep}",);

                            changed.insert(member.package_id());

                            let missing_feat = FeatureId::new(dep, missing_feat);
                            let missing_feat_ix = *fg.nodes.get(&missing_feat).unwrap();
                            fg.graph.add_edge(member_ix, missing_feat_ix, Dep::Always);
                            next = Some(missing_feat_ix);
                            made_changes = true;
                            continue 'dependency;
                        }
                    }
                }
            }

            if made_changes {
                made_changes = false;
                next = Some(member_ix);
                continue 'dependency;
            }
        }
    }

    let mut changeset: Changeset = BTreeMap::new();

    for member_id in changed {
        let member = package_graph.metadata(member_id)?;
        //        let member_ix = fg2.nodes.get(&member.default_feature_id()).unwrap();
        let member_ix = fg.nodes.get(&FeatureId::base(member.id())).unwrap(); // .default_feature_id()).unwrap();

        let member_entry = changeset.entry(member_id).or_default();

        for dep_ix in fg
            .graph
            .neighbors_directed(*member_ix, EdgeDirection::Outgoing)
        {
            if fg.graph[dep_ix] != FeatKind::External {
                continue;
            }
            let dep = fg.features.get(&dep_ix).unwrap();

            if let Some(feat) = dep.feature() {
                member_entry
                    .entry(dep.package_id())
                    .or_default()
                    .insert(feat);
            }

            if feature_graph
                .metadata(FeatureId::new(dep.package_id(), "default"))
                .is_err()
                || workspace_feats
                    .get(dep.package_id())
                    .map_or(false, |s| s.contains("default"))
            {
                member_entry
                    .entry(dep.package_id())
                    .or_default()
                    .insert("default");
            }
        }
    }

    Ok(changeset)
}

// num-bigint default on textual

pub fn apply(
    package_graph: &PackageGraph,
    dry: bool,
    lock: bool,
    meta: &Metadata,
) -> anyhow::Result<()> {
    let map = get_changeset2(package_graph, meta)?;
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

fn transitive_reduction<N, E>(graph: &mut Graph<N, E>) {
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

/*
fn dump(fg: &FGraph) -> anyhow::Result<()> {
    use tempfile::NamedTempFile;
    let mut file = NamedTempFile::new()?;
    dot::render(&FG(&fg.graph), &mut file)?;
    std::process::Command::new("xdot")
        .args([file.path()])
        .output()?;
    Ok(())
}*/

fn dump(fg: &FeatGraph2) -> anyhow::Result<()> {
    use tempfile::NamedTempFile;
    let mut file = NamedTempFile::new()?;
    dot::render(&fg, &mut file)?;
    //dot::render(&fg, &mut std::io::stdout())?;
    //    todo!("{:?}", s);
    std::process::Command::new("xdot")
        .args([file.path()])
        .output()?;
    Ok(())
}
