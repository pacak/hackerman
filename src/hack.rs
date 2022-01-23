use guppy::graph::feature::{FeatureGraph, FeatureId};
use guppy::graph::PackageMetadata;
use guppy::graph::{feature::StandardFeatures, DependencyDirection, PackageGraph};
use guppy::platform::{Platform, PlatformStatus};
use guppy::{DependencyKind, PackageId};
use petgraph::adj::NodeIndex;
use petgraph::algo::toposort;
use petgraph::algo::tred::{dag_to_toposorted_adjacency_list, dag_transitive_reduction_closure};
use petgraph::graph::Node;
use petgraph::visit::{
    Dfs, DfsPostOrder, EdgeRef, IntoNeighborsDirected, IntoNodeIdentifiers, Reversed, Visitable,
    Walker,
};
use petgraph::{EdgeDirection, Graph};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use tracing::{debug, info, trace, trace_span, warn};

use crate::dump::FeatDepGraph;
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

#[derive(Default, Debug)]
struct FGraph<'a> {
    pub graph: Graph<FeatureId<'a>, Pla>,
    pub nodes: BTreeMap<FeatureId<'a>, petgraph::prelude::NodeIndex>,
    //    pub crates: BTreeMap<&'a PackageId, BTreeSet<FeatureId<'a>>>,
    pub workspace: BTreeSet<FeatureId<'a>>,
}

impl<'a> FGraph<'a> {
    fn feat_index(&mut self, fid: FeatureId<'a>) -> petgraph::prelude::NodeIndex {
        *self
            .nodes
            .entry(fid)
            .or_insert_with(|| self.graph.add_node(fid))
    }

    fn extend_local_feats(
        &mut self,
        start_fid: FeatureId<'a>,
        feature_graph: &FeatureGraph<'a>,
    ) -> anyhow::Result<()> {
        let start_ix = self.feat_index(start_fid);

        for feat in feature_graph
            .query_forward([start_fid])?
            .resolve_with_fn(|_query, _link| false)
            .feature_ids(DependencyDirection::Forward)
        {
            if feat == start_fid {
                continue;
            }
            let feat_ix = self.feat_index(feat);
            if !self.graph.contains_edge(start_ix, feat_ix) {
                debug_assert!(start_ix != feat_ix);
                self.graph.add_edge(start_ix, feat_ix, Pla::Always);
                self.extend_local_feats(feat, feature_graph).unwrap();
            }
        }

        Ok(())
    }

    fn add_workspace_member(
        &mut self,
        pkg: PackageMetadata<'a>,
        features: &FeatureGraph<'a>,
    ) -> anyhow::Result<()> {
        let pkg_fid = pkg.default_feature_id();
        self.workspace.insert(pkg_fid);
        self.extend_local_feats(pkg_fid, features)
    }

    fn next_dependency(&self, pg: &PackageGraph) -> Option<petgraph::prelude::NodeIndex> {
        for ws_fid in self.workspace.iter() {
            let &ws_ix = self.nodes.get(ws_fid)?;

            let mut dfs = Dfs::new(&self.graph, ws_ix);
            while let Some(child_ix) = dfs.next(&self.graph) {
                let child_feat = self.graph[child_ix];
                if !pg.metadata(child_feat.package_id()).unwrap().in_workspace() {
                    //                if !self.workspace.contains(&child_feat) {
                    return Some(child_ix);
                }
            }
        }
        None
    }

    fn all_dependencies(
        &'a self,
        start: FeatureId<'a>,
    ) -> impl Iterator<Item = (petgraph::prelude::NodeIndex, FeatureId<'a>)> {
        let ix = *self.nodes.get(&start).unwrap();

        Dfs::new(&self.graph, ix)
            .iter(&self.graph)
            .map(|i| (i, self.graph[i]))
    }

    fn minimize<'b>(
        &self,
        mut set: BTreeSet<&'b PackageId>,
        package_graph: &'a PackageGraph,
    ) -> BTreeSet<&'b PackageId> {
        let mut p2i = BTreeMap::new();
        let mut i2p = BTreeMap::new();

        for pid in set.iter() {
            let p = package_graph.metadata(pid).unwrap();
            let i = self.nodes.get(&p.default_feature_id()).unwrap();
            p2i.insert(p.id(), i);
            i2p.insert(i, p.id());
        }

        let graph = Reversed(&self.graph);

        for (pid, &&ix) in p2i.iter() {
            for p in Dfs::new(&graph, ix).iter(&graph) {
                if p == ix {
                    continue;
                }
                if let Some(child) = i2p.get(&p) {
                    trace!("{pid} subsumes {child}");
                    set.remove(*child);
                }
            }
        }
        set
    }
}

pub fn get_changeset2(package_graph: &PackageGraph) -> anyhow::Result<Changeset> {
    let feature_graph = package_graph.feature_graph();
    let here = Platform::current()?;

    let mut fg = FGraph::default();

    for pkt in package_graph.workspace().iter() {
        fg.add_workspace_member(pkt, &feature_graph)?;
    }

    let mut workspace_feats = BTreeMap::new();

    feature_graph
        .query_workspace(StandardFeatures::Default)
        .resolve_with_fn(|_query, link| {
            // first we try to figure out if we want to follow this link.
            // dev links outside of the workspace are ignored
            // otherwise links are followed depending on the Platform

            let in_workspace = link.from().package().in_workspace();

            let cond = match follow(&here, link.normal()).or_else(|| follow(&here, link.build())) {
                Some(cond) => cond,
                None => return false,
            };

            fg.extend_local_feats(link.to().feature_id(), &feature_graph)
                .unwrap();

            if in_workspace {
                if let Some(feat) = link.to().feature_id().feature() {
                    workspace_feats
                        .entry(link.from().package_id())
                        .or_insert_with(BTreeMap::new)
                        .entry(link.to().package_id())
                        .or_insert_with(BTreeSet::new)
                        .insert(feat);
                }
            }

            let from = fg.feat_index(link.from().feature_id());
            let to = fg.feat_index(link.to().feature_id());

            /*

            trace!(
                "{:3}: -> {:3}: {}-> {} / {cond:?}",
                from.index(),
                to.index(),
                link.from().feature_id(),
                link.to().feature_id()
            );*/
            fg.graph.add_edge(from, to, cond);

            true
        });

    //    transitive_reduction(&mut fg.graph);

    let mut to_add = BTreeMap::new();

    /*
    let mut member_package_dependencies = BTreeMap::new();

    for member in package_graph.workspace().iter() {
        let member_deps = fg
            .all_dependencies(member.default_feature_id())
            .map(|(_, feat)| feat.package_id().clone())
            .collect::<BTreeSet<_>>();
        member_package_dependencies.insert(member.id().clone(), member_deps);
    }*/

    loop {
        let mut change = false;
        let indices = fg.nodes.values().copied().collect::<Vec<_>>();
        for &feat_ix in indices.iter() {
            //    while let Some(feat_ix) = fg.next_dependency(package_graph) {
            let feat_id = fg.graph[feat_ix];
            let feat_pkg = feat_id.package_id();

            let feat = package_graph.metadata(feat_pkg)?;
            if feat.in_workspace() {
                continue;
            }
            assert!(!feat.in_workspace(), "{feat:?}");

            debug!("\nchecking for {feat_id}");

            for member in package_graph.workspace().iter() {
                let member_name = member.name();

                /*
                if !member_package_dependencies
                    .get(member.id())
                    .unwrap()
                    .contains(feat_pkg)
                {
                    trace!("{member_name} doesn't use pkg with {feat_id}, ignoring");
                    continue;
                }*/

                if fg
                    .all_dependencies(member.default_feature_id())
                    .any(|(_, feat)| feat == feat_id)
                {
                    trace!("{member_name} uses {feat_id}, ignoring");
                    continue;
                }

                if !fg
                    .all_dependencies(member.default_feature_id())
                    .any(|(_, feat)| feat.package_id() == feat_pkg)
                {
                    trace!("{member_name} doesn't use {feat_pkg}, ignoring");
                    continue;
                }

                info!("{member_name} uses {feat_pkg} but not {feat_id}");
                to_add
                    .entry(feat_id)
                    .or_insert_with(BTreeSet::new)
                    .insert(member.id());

                let member_id = *fg.nodes.get(&member.default_feature_id()).unwrap();
                info!("Added edge {member_id:?} -> {feat_ix:?}");
                fg.graph.add_edge(member_id, feat_ix, Pla::Always);
                change = true;
            }

            //        debug!("Removing node by splice {feat_id}/{feat_ix:?}");
            //        fg.graph.remove_node(feat_ix);
        }
        if !change {
            break;
        }
    }

    let mut changeset: Changeset = BTreeMap::new();

    for (feat, workspace_crates) in to_add.into_iter() {
        let workspace_crates = fg.minimize(workspace_crates, package_graph);
        for ws_crate in workspace_crates {
            let ws_crate_deps = changeset.entry(ws_crate).or_insert_with(BTreeMap::new);

            if let Some(feat_name) = feat.feature() {
                let dependency_feats = ws_crate_deps
                    .entry(feat.package_id())
                    .or_insert_with(BTreeSet::new);

                dependency_feats.insert(feat_name);
            } else {
                unreachable!("this doesn't make sense")
            }
        }
    }

    for (pkt, changes) in changeset.iter_mut() {
        println!("{pkt}");
        for (a, b) in changes.iter_mut() {
            println!("\t{a} {b:?}");
            if let Some(existing) = workspace_feats.get(pkt) {
                if let Some(for_feat) = existing.get(a) {
                    println!("\t\t{for_feat:?}");
                    b.extend(for_feat);
                }
            }
        }
    }

    Ok(changeset)
}

// num-bigint default on textual

pub fn apply(package_graph: &PackageGraph, dry: bool, lock: bool) -> anyhow::Result<()> {
    let kind = DependencyKind::Normal;
    let map = get_changeset2(package_graph)?;
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
