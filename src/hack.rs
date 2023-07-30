#![allow(clippy::similar_names)]

use crate::{
    feat_graph::{Feat, FeatGraph, Pid},
    metadata::DepKindInfo,
    source::ChangePackage,
    toml::set_dependencies,
};
use cargo_metadata::Metadata;
use cargo_platform::Cfg;
use petgraph::{
    graph::NodeIndex,
    visit::{Dfs, DfsPostOrder, EdgeFiltered, EdgeRef, NodeFiltered, VisitMap, Walker},
};
use std::collections::{BTreeMap, BTreeSet};
use tracing::{debug, info, trace, warn};

fn force_config(var: &mut bool, name: &str, meta: &serde_json::Value) -> Option<()> {
    *var = meta.get("hackerman")?.get(name)?.as_bool()?;
    Some(())
}

pub fn hack(
    dry: bool,
    mut lock: bool,
    mut no_dev: bool,
    meta: &Metadata,
    triplets: Vec<&str>,
    cfgs: Vec<Cfg>,
) -> anyhow::Result<bool> {
    force_config(&mut lock, "lock", &meta.workspace_metadata);
    force_config(&mut no_dev, "no-dev", &meta.workspace_metadata);

    let mut fg = FeatGraph::init(meta, triplets, cfgs)?;
    let changeset = get_changeset(&mut fg, no_dev)?;
    let has_changes = !changeset.is_empty();

    if dry {
        if changeset.is_empty() {
            println!("Features are unified as is");
            return Ok(false);
        }
        println!("Hackerman would like to set those features for following packets:");
    }

    for (member, changes) in changeset {
        let mut changeset = changes
            .into_iter()
            .map(|change| ChangePackage::make(member, change))
            .collect::<anyhow::Result<Vec<_>>>()?;

        if dry {
            changeset.sort_by(|a, b| a.name.cmp(&b.name));
            let path = &member.package().manifest_path;
            println!("{path}");
            for change in changeset {
                let t = match change.ty {
                    Ty::Dev => "dev ",
                    Ty::Norm => "",
                };
                println!(
                    "\t{} {} {}: {t}{:?}",
                    change.name, change.version, change.source, change.feats
                );
            }
        } else {
            let path = &member.package().manifest_path;
            set_dependencies(path, lock, &changeset)?;
        }
    }

    if dry && has_changes {
        anyhow::bail!("Features are not unified");
    }

    Ok(has_changes)
}

pub struct FeatChange<'a> {
    /// package id of the dependency we are adding
    pub pid: Pid<'a>,

    /// dependency type - dev or normal
    pub ty: Ty,

    /// Crate needs renaming
    pub rename: bool,

    /// Features to add
    pub features: BTreeSet<String>,
}

type FeatChanges<'a> = BTreeMap<Pid<'a>, Vec<FeatChange<'a>>>;
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
    /// all targets, normal and builds
    AllTargets,
    /// current target only
    Target,
    /// current target only, normal and build dependencies globally, dev dependencies for workspace
    DevTarget,
    NoDev,
    MemberDev(Pid<'a>),
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
    let mut to_visit = Vec::new();
    let mut added = BTreeSet::new();

    let g = EdgeFiltered::from_fn(&fg.features, |e| {
        // last_edge.set(Some(e));
        match filter {
            Collect::AllTargets => true,
            Collect::Target | Collect::NoDev | Collect::DevTarget | Collect::MemberDev(_) => e
                .weight()
                .satisfies(fg.features[e.source()], filter, &fg.platforms, &fg.cfgs),
        }
    });

    loop {
        while let Some(ix) = dfs.next(&g) {
            if let Some(fid) = fg.features[ix].fid() {
                if let Some(parent) = fg.fid_cache.get(&fid.get_base()) {
                    to.entry(*parent).or_insert_with(BTreeSet::new).insert(ix);
                }
            }
        }
        for t in fg.triggers.iter() {
            let package = fg.fid_cache[&t.package.base().get_base()];
            let feature = fg.fid_cache[&t.feature]; // .unwrap();
            let weak_dep = fg.fid_cache[&t.weak_dep];
            let weak_feat = fg.fid_cache[&t.weak_feat];

            if let Some(dep) = to.get(&package) {
                if dep.contains(&feature) && dep.contains(&weak_dep) && added.insert(weak_feat) {
                    to_visit.push(weak_feat);
                }
            }
        }

        if let Some(next) = to_visit.pop() {
            dfs.move_to(next);
        } else {
            break;
        }
    }
}

#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum Ty {
    Dev,
    Norm,
}

impl Ty {
    #[must_use]
    pub const fn table_name(&self) -> &'static str {
        match self {
            Ty::Dev => "dev-dependencies",
            Ty::Norm => "dependencies",
        }
    }
}

impl std::fmt::Display for Ty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ty::Dev => f.write_str("dev"),
            Ty::Norm => f.write_str("norm"),
        }
    }
}

pub fn get_changeset<'a>(fg: &mut FeatGraph<'a>, no_dev: bool) -> anyhow::Result<FeatChanges<'a>> {
    info!("==== Calculating changeset for hack");

    //    dump(fg)?;
    let mut changed = BTreeMap::new();
    //    loop {
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
        Collect::AllTargets,
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
                }
                seen.insert(pid);

                let package = pid.package();
                let fid = if package.features.contains_key("default") {
                    pid.named("default")
                } else {
                    pid.base()
                };
                if let Some(&ix) = fg.fid_cache.get(&fid) {
                    res.push((pid, ix));
                } else {
                    warn!("unknown base in workspace: {fid:?}?");
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

            for (&dep, feats) in &deps_feats {
                if let Some(ws_feats) = raw_workspace_feats.get(&dep) {
                    if ws_feats != feats {
                        if let Some(&missing_feat) = ws_feats.difference(feats).next() {
                            info!("\t{member:?} lacks {}", fg.features[missing_feat]);

                            changed
                                .entry(member)
                                .or_insert_with(BTreeMap::default)
                                .insert((Ty::Norm, dep), ws_feats.clone());

                            let new_dep =
                                fg.add_edge(member_ix, missing_feat, false, DepKindInfo::NORMAL)?;
                            dfs.move_to(new_dep);

                            trace!("Performing one more iteration on {member:?}");
                            continue 'dependency;
                        }
                    }
                }
            }

            break;
        }

        if no_dev {
            continue;
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

            for (&dep, feats) in &dev_feats {
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

    // renames are needed when there's several dependencies from a member with the same name.
    // there can be one, two or three of them.
    let mut renames = BTreeMap::new();
    for package in &fg.workspace_members {
        use std::cell::RefCell;
        let mut deps = BTreeMap::new();
        let cell = RefCell::new(&mut deps);
        let package_index = match fg.fid_cache.get(&package.root()) {
            Some(ix) => ix,
            None => continue,
        };
        let g = EdgeFiltered::from_fn(&fg.features, |edge| {
            if fg.features[edge.target()].pid() == Some(*package) {
                true
            } else {
                if let Some(dep) = fg.features[edge.target()].pid() {
                    let dep = dep.package();
                    cell.borrow_mut()
                        .entry(dep.name.clone())
                        .or_insert_with(BTreeSet::new)
                        .insert(&dep.id);
                }
                false
            }
        });

        let mut dfs = Dfs::new(&g, *package_index);
        while dfs.next(&g).is_some() {}
        deps.retain(|_key, val| val.len() > 1);
        for (dep, _versions) in deps {
            renames
                .entry(*package)
                .or_insert_with(BTreeSet::new)
                .insert(dep);
        }
    }

    Ok(changed
        .into_iter()
        .map(|(pid, deps)| {
            let feats = deps
                .into_iter()
                .filter_map(|((ty, dep_pid), feats)| {
                    let package = fg.features[dep_pid].fid()?.pid;
                    let feats = feats
                        .iter()
                        .filter_map(|f| match fg.features[*f].fid()?.dep {
                            Feat::Base => None,
                            Feat::Named(name) => Some(name.to_string()),
                        })
                        .collect::<BTreeSet<_>>();
                    let rename = renames
                        .get(&pid)
                        .map_or(false, |names| names.contains(&package.package().name));
                    Some(FeatChange {
                        pid: package,
                        ty,
                        rename,
                        features: feats,
                    })
                })
                .collect::<Vec<_>>();
            (pid, feats)
        })
        .collect::<BTreeMap<_, _>>())
}
