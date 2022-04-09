use std::collections::{BTreeMap, BTreeSet};

use crate::feat_graph::{FeatGraph2, Pid};
use cargo_metadata::Metadata;

pub fn hack(dry: bool, lock: bool, meta: &Metadata) -> anyhow::Result<()> {
    let map = get_changeset(meta)?;

    Ok(())
}

type Changeset<'a> = BTreeMap<Pid<'a>, BTreeMap<Pid<'a>, BTreeSet<&'a str>>>;
pub fn get_changeset<'a>(meta: &'a Metadata) -> anyhow::Result<Changeset<'a>> {
    let fg2 = FeatGraph2::init(meta)?;

    //    todo!("{:?}", fg2);
    dump(&fg2)?;
    //    todo!("{:?}", meta.resolve.as_ref().unwrap().root);

    /*
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
        */

    todo!();
}

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
