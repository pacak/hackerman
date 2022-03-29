use guppy::graph::feature::FeatureId;
use guppy::graph::{feature::StandardFeatures, DependencyDirection, PackageGraph};
use guppy::{DependencyKind, PackageId};
use petgraph::visit::{Dfs, DfsPostOrder, NodeFiltered, Walker};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use tracing::{debug, info, trace, trace_span, warn};

use crate::feat_graph::{FeatGraph, Feature, Fid, Link, Pid};
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

type FeatChanges<'a> = BTreeMap<Pid<'a>, BTreeMap<Pid<'a>, BTreeSet<&'a str>>>;

pub fn get_changeset2<'a>(
    fg: &'a mut FeatGraph,
) -> anyhow::Result<(FeatChanges<'a>, &'a FeatGraph<'a>)> {
    let mut workspace_feats: BTreeMap<Pid, BTreeSet<&str>> = BTreeMap::new();

    for f in fg.features.node_weights() {
        if let Feature::External(_, Fid(pid, Some(feat))) = f {
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

                            let missing_feat = Fid(*dep, Some(missing_feat)); //FeatureId::new(dep, missing_feat);
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
}

fn get_changeset(package_graph: &PackageGraph) -> anyhow::Result<Changeset> {
    // {{{
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

// }}}

pub fn apply2(features: &FeatGraph, changeset: FeatChanges) -> anyhow::Result<()> {
    if changeset.is_empty() {
        info!("Nothing to do!");
        return Ok(());
    }

    for (member, additions) in changeset.into_iter() {
        let package = member.package();
        toml::set_dependencies2(package, additions)?;
        //        let manifest = member.package().manifest_path;
    }

    Ok(())
}

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

pub fn restore_file(path: &OsStr) -> anyhow::Result<()> {
    crate::toml::restore_dependencies(path)?;
    Ok(())
}
