use std::collections::BTreeMap;

use guppy::graph::{
    feature::{FeatureGraph, FeatureId},
    DependencyDirection,
};
use petgraph::{
    graph::{Node, NodeIndex},
    visit::NodeIndexable,
    Graph,
};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FeatKind {
    Root,
    Workspace,
    External,
}

#[derive(Copy, Clone, Debug)]
pub enum Dep {
    Always,
}

#[derive(Clone, Debug)]
pub struct FeatGraph<'a> {
    pub nodes: BTreeMap<FeatureId<'a>, NodeIndex>,
    pub graph: Graph<FeatKind, Dep>,
    pub features: BTreeMap<NodeIndex, FeatureId<'a>>,
    feature_graph: FeatureGraph<'a>,
}

fn root() -> NodeIndex {
    NodeIndex::new(0)
}

impl<'a> FeatGraph<'a> {
    pub fn init(feature_graph: FeatureGraph<'a>) -> anyhow::Result<Self> {
        let mut res = Self {
            nodes: BTreeMap::new(),
            graph: Graph::new(),
            feature_graph,
            features: BTreeMap::new(),
        };
        assert_eq!(res.graph.add_node(FeatKind::Root).index(), 0);

        for package in feature_graph.package_graph().workspace().iter() {
            //            let feat = package.default_feature_id();
            //            let feat_ix = res.feat_index(feat, FeatKind::Workspace);
            let package_ix =
                res.extend_local_feats(package.default_feature_id(), FeatKind::Workspace)?;

            res.graph.add_edge(root(), package_ix, Dep::Always);
        }

        Ok(res)
    }

    pub fn feat_index(&mut self, fid: FeatureId<'a>, kind: FeatKind) -> NodeIndex {
        *self.nodes.entry(fid).or_insert_with(|| {
            let n = self.graph.add_node(kind);
            self.features.insert(n, fid);
            n
        })
    }

    pub fn extend_local_feats(
        &mut self,
        start_fid: FeatureId<'a>,
        kind: FeatKind,
    ) -> anyhow::Result<NodeIndex> {
        let start_ix = self.feat_index(start_fid, kind);

        for feat in self
            .feature_graph
            .query_forward([start_fid])?
            .resolve_with_fn(|_query, _link| false)
            .feature_ids(DependencyDirection::Forward)
        {
            if feat == start_fid {
                continue;
            }
            let feat_ix = self.feat_index(feat, kind);
            if !self.graph.contains_edge(start_ix, feat_ix) {
                debug_assert!(start_ix != feat_ix);
                self.graph.add_edge(start_ix, feat_ix, Dep::Always);
                self.extend_local_feats(feat, kind).unwrap();
            }
        }

        Ok(start_ix)
    }
}
