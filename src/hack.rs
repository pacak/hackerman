use petgraph::visit::{Dfs, DfsPostOrder, NodeFiltered, Walker};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
};

use crate::feat_graph::{dump, FeatGraph, Feature, Fid, Link, Pid};
//use crate::{query::*, toml};
use cargo_metadata::Metadata;
use tracing::{debug, error, info, warn};

pub fn hack(dry: bool, lock: bool, meta: &Metadata, triplets: Vec<&str>) -> anyhow::Result<()> {
    let mut fg = FeatGraph::init(meta, triplets)?;

    dump(&fg)?;
    //    let changeset = get_changeset(&mut fg)?;

    //    todo!();
    Ok(())
}

type FeatChanges<'a> = BTreeMap<Pid<'a>, BTreeMap<Pid<'a>, BTreeSet<&'a str>>>;

pub fn get_changeset<'a>(
    fg: &'a mut FeatGraph,
) -> anyhow::Result<(FeatChanges<'a>, &'a FeatGraph<'a>)> {
    /*
    let mut workspace_feats: BTreeMap<Pid, BTreeSet<&str>> = BTreeMap::new();

    for f in fg.features.node_weights() {
        if let Feature::External(_, Fid { pid, dep: Some(feat) }) = f {
            workspace_feats
                .entry(*pid)
                .or_insert_with(BTreeSet::new)
                .insert(feat);
        }
    }
    let mut changed = BTreeMap::new();

    let workspace_only_graph =
        NodeFiltered::from_fn(&fg.features, |node| fg.features[node].is_workspace());

    let members_dfs_postorder = DfsPostOrder::new(&workspace_only_graph, fg.root)
        .iter(&workspace_only_graph)
        .collect::<Vec<_>>();
    for member_ix in members_dfs_postorder {
        let member = match fg.features[member_ix].pid() {
            Some(pid) => pid,
            None => continue,
        };

        info!("Checking {member:?}");

        let mut deps_feats = BTreeMap::new();

        let mut next = Some(member_ix);
        let mut dfs = Dfs::new(&fg.features, member_ix);
        let mut made_changes = false;
        'dependency: while let Some(next_item) = next.take() {
            dfs.move_to(next_item);
            // DFS traverse of the current member and all added
            while let Some(feat_ix) = dfs.next(&fg.features) {
                let feat_id: Feature<'a> = fg.features[feat_ix];

                let pkg_id = feat_id.pid().unwrap(); // package_id().unwrap();
                let entry = deps_feats.entry(pkg_id).or_insert_with(BTreeSet::new);

                if let Some(feat) = feat_id.feature() {
                    entry.insert(feat);
                }
            }

            for (dep, feats) in deps_feats.iter() {
                if let Some(ws_feats) = workspace_feats.get(dep) {
                    if ws_feats != feats {
                        if let Some(missing_feat) = ws_feats.difference(feats).next() {
                            info!("\t{missing_feat:?} is missing from {dep:?}",);

                            changed
                                .entry(member)
                                .or_insert_with(BTreeMap::default)
                                .entry(*dep)
                                .or_insert_with(BTreeSet::default)
                                .insert(*missing_feat);

                            let missing_feat = Fid { pid: *dep, dep: Some(missing_feat) }; //FeatureId::new(dep, missing_feat);
                            let missing_feat_ix = *fg.fids.get(&missing_feat).unwrap();
                            fg.features
                                .add_edge(member_ix, missing_feat_ix, Link::always());
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

    for (k, v) in changed.iter() {
        debug!("{k:?}");
        for (kk, vv) in v.iter() {
            debug!("\t{kk:?}: {vv:?}");
        }
    }

    //    todo!("{:?}", changed);
    Ok((changed, fg))
    //    todo!();
    //
    */

    todo!();
}

/*

fn get_changeset(package_graph: &PackageGraph) -> anyhow::Result<Changeset> {
    // {{{
    let kind = DependencyKind::Normal;


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

    <<<<<<< HEAD
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
    =======
    >>>>>>> 62f396f
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

    todo!();
}

            */
// }}}

pub fn apply2(features: &FeatGraph, changeset: FeatChanges) -> anyhow::Result<()> {
    if changeset.is_empty() {
        info!("Nothing to do!");
        return Ok(());
    }

    for (member, additions) in changeset.into_iter() {
        let package = member.package();
        //        toml::set_dependencies2(package, additions)?;
        //        let manifest = member.package().manifest_path;
    }

    Ok(())
}

/*
pub fn apply(package_graph: &PackageGraph, dry: bool, lock: bool) -> anyhow::Result<()> {
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

*/
/*
pub fn restore(package_graph: PackageGraph) -> anyhow::Result<()> {
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
*/
pub fn restore_file(path: &OsStr) -> anyhow::Result<()> {
    //    crate::toml::restore_dependencies(path)?;
    Ok(())
}
