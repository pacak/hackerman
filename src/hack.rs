use crate::{
    feat_graph::{Feat, FeatGraph, Fid, Pid},
    metadata::DepKindInfo,
    source::*,
    toml::set_dependencies,
};
use cargo_metadata::Metadata;
use cargo_platform::Cfg;
use petgraph::{
    graph::NodeIndex,
    visit::{
        Dfs, DfsPostOrder, EdgeFiltered, EdgeRef, IntoEdgeReferences, NodeFiltered, VisitMap,
        Walker,
    },
};
use std::collections::{BTreeMap, BTreeSet};
use tracing::{debug, info, trace, warn};

fn force_lock(lock: &mut bool, meta: &serde_json::Value) -> Option<()> {
    *lock = meta.get("hackerman")?.get("lock")?.as_bool()?;
    Some(())
}

pub fn hack(
    dry: bool,
    mut lock: bool,
    meta: &Metadata,
    triplets: Vec<&str>,
    cfgs: Vec<Cfg>,
) -> anyhow::Result<()> {
    force_lock(&mut lock, &meta.workspace_metadata);

    let mut fg = FeatGraph::init(meta, triplets, cfgs)?;
    let changeset = get_changeset(&mut fg)?;
    let has_changes = !changeset.is_empty();

    if dry {
        if changeset.is_empty() {
            println!("Features are unified as is");
            return Ok(());
        } else {
            println!("Hackerman would like to set those features for following packets:");
        }
    }

    for (pid, changes) in changeset.into_iter() {
        let mut changeset = changes
            .into_iter()
            .map(|(fid, ty, feats)| ChangePackage::make(pid, fid, ty, feats))
            .collect::<Vec<_>>();

        if dry {
            changeset.sort_by(|a, b| a.name.cmp(&b.name));
            let path = &pid.package().manifest_path;
            println!("{path}");
            for change in changeset {
                let t = match change.ty {
                    Ty::Dev => "dev ",
                    Ty::Norm => "",
                };
                println!(
                    "\t{} {}: {}{:?}",
                    change.name, change.source, t, change.feats
                )
            }
        } else {
            let path = &pid.package().manifest_path;
            set_dependencies(path, lock, &changeset)?;
        }
    }

    if dry && has_changes {
        anyhow::bail!("Features are not unified");
    }

    Ok(())
}

type FeatChanges<'a> = BTreeMap<Pid<'a>, Vec<(Fid<'a>, Ty, BTreeSet<String>)>>;
type DetachedDepTree = BTreeMap<NodeIndex, BTreeSet<NodeIndex>>;

fn show_detached_dep_tree(tree: &DetachedDepTree, fg: &FeatGraph) -> &'static str {
    let mut t = tree.iter().collect::<Vec<_>>();

    t.sort_by(|a, b| fg.features[*a.0].fid().cmp(&fg.features[*b.0].fid()));

    for (&package, feats) in t {
        //tree.iter() {
        let package = fg.features[package];
        print!("{package}\n\t");
        for feature in feats.iter().copied() {
            let feature = fg.features[feature];
            let fid = feature.fid().unwrap();
            assert_eq!(package.fid().unwrap().pid, fid.pid);
            print!("{} ", fid.dep);
        }
        println!();
    }
    ""
}

#[derive(Debug, Clone, Copy)]
pub enum Collect<'a> {
    /// all targets, normal and builds - from everywhere, dev - workspace only
    All,
    /// current target only, else same as all
    Target,
    NoDev,
    MemberDev(Pid<'a>),
}

impl<'a> Collect<'a> {
    pub fn is_all(&self) -> bool {
        match self {
            Collect::All => true,
            Collect::Target | Collect::NoDev | Collect::MemberDev(_) => false,
        }
    }
}

// we are doing 4 types of passes:
// 1. everything for all the targets
// 2. everything for this target - this is used to filter the first one
// 3. starting from a workspace member, no dev
// 4. starting from a workspace member, dev for that membe only

fn collect_features_from<M>(
    dfs: &mut Dfs<NodeIndex, M>,
    fg: &FeatGraph,
    to: &mut DetachedDepTree,
    filter: Collect,
) where
    M: VisitMap<NodeIndex>,
{
    let g = EdgeFiltered::from_fn(&fg.features, |e| match filter {
        Collect::All => true,
        Collect::Target | Collect::NoDev | Collect::MemberDev(_) => {
            e.weight()
                .satisfies(fg.features[e.source()], filter, &fg.platforms, &fg.cfgs)
        }
    });

    while let Some(ix) = dfs.next(&g) {
        if let Some(fid) = fg.features[ix].fid() {
            if let Some(parent) = fg.fid_cache.get(&fid.base()) {
                to.entry(*parent).or_insert_with(BTreeSet::new).insert(ix);
            }
        }
    }
}

#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum Ty {
    Dev,
    Norm,
}

pub fn get_changeset<'a>(fg: &mut FeatGraph<'a>) -> anyhow::Result<FeatChanges<'a>> {
    info!("==== Calculating changeset for hack");

    //    dump(fg)?;
    let mut changed = BTreeMap::new();
    loop {
        // First we collect all the named feats. The idea if some crate depends on
        // the base feature (key) it should depend on all the named features of this
        // crate (values).

        // DetachedDepTree is used to avoid fighting the borrow checker.
        // indices correspond to features in graph
        let mut raw_workspace_feats: DetachedDepTree = BTreeMap::new();
        collect_features_from(
            &mut Dfs::new(&fg.features, fg.root),
            fg,
            &mut raw_workspace_feats,
            Collect::All,
        );

        // For reasons unknown cargo resolves dependencies for all the targets including those
        // never be used. While we have to care about features added at this step - we can skip
        // them for crates that never will be used - such as winapi on linux. second pass does
        // that.
        let mut filtered_workspace_feats = BTreeMap::new();
        collect_features_from(
            &mut Dfs::new(&fg.features, fg.root),
            fg,
            &mut filtered_workspace_feats,
            Collect::Target,
        );
        raw_workspace_feats.retain(|k, _| filtered_workspace_feats.contains_key(k));

        info!(
            "Accumulated workspace dependencies{}",
            show_detached_dep_tree(&raw_workspace_feats, fg)
        );
        let members = {
            let workspace_only_graph =
                NodeFiltered::from_fn(&fg.features, |node| fg.features[node].is_workspace());

            // all the "feature" nodes that belong to the workspace
            let members_dfs_postorder = DfsPostOrder::new(&workspace_only_graph, fg.root)
                .iter(&workspace_only_graph)
                .collect::<Vec<_>>();

            // only feature "roots" nodes, deduplicated
            let mut res = Vec::new();
            let mut seen = BTreeSet::new();
            for member in members_dfs_postorder {
                if let Some(pid) = fg.features[member].pid() {
                    if seen.contains(&pid) {
                        continue;
                    } else {
                        seen.insert(pid);
                    }
                    let package = pid.package();
                    let fid = if package.features.contains_key("default") {
                        pid.named("default")
                    } else {
                        pid.base()
                    };
                    if let Some(&ix) = fg.fid_cache.get(&fid) {
                        res.push((pid, ix))
                    } else {
                        warn!("unknown base in workspace: {fid:?}?")
                    }
                }
            }
            res
        };

        for (member, member_ix) in members.iter().copied() {
            info!("==== Checking {member:?}");

            // For every workspace member we start collecting features it uses, similar to
            // workspace_feats above

            let mut dfs = Dfs::new(&fg.features, member_ix);
            let mut deps_feats = BTreeMap::new();
            'dependency: loop {
                collect_features_from(&mut dfs, fg, &mut deps_feats, Collect::NoDev);

                debug!(
                    "Accumulated deps for {:?} are as following:{}",
                    member.package().name,
                    show_detached_dep_tree(&deps_feats, fg),
                );

                for (&dep, feats) in deps_feats.iter() {
                    if let Some(ws_feats) = raw_workspace_feats.get(&dep) {
                        if ws_feats != feats {
                            if let Some(&missing_feat) = ws_feats.difference(feats).next() {
                                info!("\t{member:?} lacks {}", fg.features[missing_feat]);

                                changed
                                    .entry(member)
                                    .or_insert_with(BTreeMap::default)
                                    .insert((Ty::Norm, dep), ws_feats.clone());

                                let new_dep = fg.add_edge(
                                    member_ix,
                                    missing_feat,
                                    false,
                                    DepKindInfo::NORMAL,
                                )?;
                                dfs.move_to(new_dep);

                                trace!("Performing one more iteration on {member:?}");
                                continue 'dependency;
                            }
                        }
                    }
                }

                break;
            }

            // at this point dep_feats contains all the normal features used by {member}.
            // we'll use it to filter dep dependencies if any.

            if !member
                .package()
                .dependencies
                .iter()
                .any(|d| d.kind == cargo_metadata::DependencyKind::Development)
            {
                debug!("No dev dependencies for {member:?}, skipping");
                continue;
            }

            let mut dfs = Dfs::new(&fg.features, member_ix);
            let mut dev_feats = BTreeMap::new();
            'dev_dependency: loop {
                // DFS traverse of the current member and everything below it
                collect_features_from(&mut dfs, fg, &mut dev_feats, Collect::MemberDev(member));

                dev_feats.retain(|key, _val| filtered_workspace_feats.contains_key(key));

                debug!(
                    "Accumulated dev deps for {:?} are as following:{}",
                    member.package().name,
                    show_detached_dep_tree(&dev_feats, fg),
                );

                for (&dep, feats) in dev_feats.iter() {
                    if let Some(ws_feats) = raw_workspace_feats.get(&dep) {
                        if ws_feats != feats {
                            if let Some(&missing_feat) = ws_feats.difference(feats).next() {
                                debug!("\t{member:?} lacks dev {}", fg.features[missing_feat]);

                                changed
                                    .entry(member)
                                    .or_insert_with(BTreeMap::default)
                                    .insert((Ty::Dev, dep), ws_feats.clone());

                                let new_dep =
                                    fg.add_edge(member_ix, missing_feat, false, DepKindInfo::DEV)?;
                                dfs.move_to(new_dep);

                                trace!("Performing one more dev iteration on {member:?}");
                                continue 'dev_dependency;
                            }
                        }
                    }
                }

                break;
            }
        }

        // to do triggers we traverse from each triggering package, collect all the
        // package dependencies and locally enabled features then look for
        // triggers that satisfy the conditions and not enabled yet then add those,
        // remove them from fg and do one more pass
        let mut weak_deps = Vec::new();
        for (pid, triggers) in fg.triggers.iter_mut() {
            let mut local_fids = BTreeSet::new();

            let mut remote_pids = BTreeSet::new();
            let mut remote_fids = BTreeSet::new();

            let sub = EdgeFiltered::from_fn(&fg.features, |edge| {
                fg.features[edge.source()]
                    .fid()
                    .map_or(false, |fid| fid.pid == *pid)
            });

            for edge in sub.edge_references() {
                if let Some(fid) = fg.features[edge.target()].fid() {
                    if fid.pid == *pid {
                        local_fids.insert(fid);
                    } else {
                        remote_pids.insert(fid.pid);
                        remote_fids.insert(fid);
                    }
                }
            }

            if pid.package().features.contains_key("default") {
                local_fids.insert(pid.named("default"));
            }

            triggers.retain(|trigger| {
                if local_fids.contains(&trigger.feature) && remote_pids.contains(&trigger.weak_dep)
                {
                    if !remote_fids.contains(&trigger.weak_feat) {
                        weak_deps.push((trigger.package, trigger.weak_feat));
                    }
                    false
                } else {
                    true
                }
            });
        }

        if weak_deps.is_empty() {
            break;
        } else {
            debug!("Weak dependencies add {} new links", weak_deps.len());
            for (a, b) in weak_deps {
                fg.add_edge(a, b, false, DepKindInfo::NORMAL)?;
            }
        }
    }

    Ok(changed
        .into_iter()
        .map(|(pid, deps)| {
            let feats = deps
                .into_iter()
                .filter_map(|((ty, pid), feats)| {
                    let package = fg.features[pid].fid()?;
                    let feats = feats
                        .iter()
                        .filter_map(|f| match fg.features[*f].fid().unwrap().dep {
                            Feat::Base => None,
                            Feat::Named(name) => Some(name.to_string()),
                        })
                        .collect::<BTreeSet<_>>();
                    Some((package, ty, feats))
                })
                .collect::<Vec<_>>();
            (pid, feats)
        })
        .collect::<BTreeMap<_, _>>())
}
