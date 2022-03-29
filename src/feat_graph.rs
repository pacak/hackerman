use cargo_metadata::{DepKindInfo, Dependency, Metadata, Node, Package, PackageId};
use dot::{GraphWalk, Labeller};
use petgraph::graph::{EdgeIndex, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::Graph;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use tracing::{debug, info, trace};

#[derive(Copy, Clone, Ord, PartialEq, Eq, PartialOrd, Debug)]
pub enum Feature<'a> {
    Root(Platforms),
    Workspace(Platforms, Fid<'a>),
    External(Platforms, Fid<'a>),
}

impl<'a> Feature<'a> {
    pub fn fid(&self) -> Option<Fid<'a>> {
        match self {
            Feature::Root(_) => None,
            Feature::Workspace(_, fid) | Feature::External(_, fid) => Some(*fid),
        }
    }

    pub fn platforms(&self) -> Platforms {
        match self {
            Feature::Root(p) => *p,
            Feature::Workspace(p, _) | Feature::External(p, _) => *p,
        }
    }

    pub fn platforms_mut(&mut self) -> &mut Platforms {
        match self {
            Feature::Root(p) => p,
            Feature::Workspace(p, _) | Feature::External(p, _) => p,
        }
    }

    pub fn pid(&self) -> Option<Pid<'a>> {
        self.fid().map(|fid| fid.0)
    }

    pub fn package_id(&self) -> Option<&PackageId> {
        let Pid(pid, meta) = self.pid()?;
        Some(&meta.packages[pid].id)
    }

    pub fn feature(&self) -> Option<&'a str> {
        let fid = self.fid()?;
        fid.1
    }

    pub fn is_workspace(&self) -> bool {
        match self {
            Feature::Root(_) | Feature::Workspace(_, _) => true,
            Feature::External(_, _) => false,
        }
    }

    pub fn is_root(&self) -> bool {
        match self {
            Feature::Root(_) => true,
            Feature::Workspace(_, _) | Feature::External(_, _) => false,
        }
    }
}

pub struct FeatGraph<'a> {
    pub workspace_members: BTreeSet<Pid<'a>>,
    pub features: Graph<Feature<'a>, Link<'a>>,
    pub fids: BTreeMap<Fid<'a>, NodeIndex>,
    pub cache: BTreeMap<&'a PackageId, Pid<'a>>,
    pub meta: &'a Metadata,
    pub root: NodeIndex,
    pub library_renames: BTreeMap<&'a PackageId, &'a str>,
    pub platforms: Vec<&'a str>,
}

// there are some very strange ideas about what is a valid crate is name and how to compare
// them out there
fn name_cmp(a: &str, b: &str) -> bool {
    a.chars()
        .zip(b.chars())
        .all(|(l, r)| l.to_ascii_lowercase() == r.to_ascii_lowercase() || (l == '-' && r == '_'))
}

fn find_dep_by_name<'a>(deps: &'a [Dependency], name: &'a str) -> anyhow::Result<&'a Dependency> {
    deps.iter()
        .find(|d| match d.rename.as_ref() {
            Some(rename) => name_cmp(rename, name),
            None => name_cmp(&d.name, name),
        })
        .ok_or_else(|| anyhow::anyhow!("No dependency named {name}"))
}

impl<'a> FeatGraph<'a> {
    pub fn fid_index(&mut self, fid: Fid<'a>) -> NodeIndex {
        *self.fids.entry(fid).or_insert_with(|| {
            let ps = Platforms::default();
            if self.workspace_members.contains(&fid.0) {
                self.features.add_node(Feature::Workspace(ps, fid))
            } else {
                self.features.add_node(Feature::External(ps, fid))
            }
        })
    }

    pub fn init(meta: &'a Metadata, platforms: Vec<&'a str>) -> anyhow::Result<Self> {
        let resolves = &meta
            .resolve
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Couldn't resolve the depdendencies"))?
            .nodes;

        let cache = meta
            .packages
            .iter()
            .enumerate()
            .map(|(ix, package)| (&package.id, Pid(ix, meta)))
            .collect::<BTreeMap<_, _>>();

        let mut features = Graph::new();
        let root = features.add_node(Feature::Root(Platforms::new(platforms.len())));

        let mut library_renames: BTreeMap<&PackageId, &str> = BTreeMap::new();
        for p in meta.packages.iter() {
            if let Some(target) = p.targets.iter().find(|t| t.kind == ["lib"]) {
                if target.name != p.name {
                    library_renames.insert(&p.id, &p.name);
                }
            }
        }

        let mut graph = FeatGraph {
            workspace_members: meta
                .workspace_members
                .iter()
                .flat_map(|pid| cache.get(pid))
                .copied()
                .collect::<BTreeSet<_>>(),
            features,
            root,
            platforms,
            fids: BTreeMap::new(),
            //            pids: BTreeMap::new(),
            library_renames,
            cache,
            meta,
        };

        for (ix, (package, deps)) in meta.packages.iter().zip(resolves.iter()).enumerate() {
            assert_eq!(package.id, deps.id);
            graph.add_package(ix, package, deps)?;
        }
        info!("Filling in patform info");
        graph.fill_in_platforms()?;
        info!("Optimizing");
        graph.optimize()?;
        info!("Rebuilding feature id cache");
        graph.fids.clear();
        for node in graph.features.node_indices() {
            if let Some(fid) = graph.features[node].fid() {
                graph.fids.insert(fid, node);
            }
        }

        Ok(graph)
    }

    fn fill_in_platforms(&mut self) -> anyhow::Result<()> {
        let mut to_visit = vec![self.root];
        let mut edges = Vec::new();
        let mut checked_children = BTreeSet::new();
        while let Some(source) = to_visit.pop() {
            if checked_children.contains(&source) {
                continue;
            } else {
                checked_children.insert(source);
            }

            trace!("{:?}", self.features[source].pid());
            let platforms = self.features[source].platforms();

            edges.extend(
                self.features
                    .edges_directed(source, petgraph::EdgeDirection::Outgoing)
                    .map(|er| (er.id(), er.target())),
            );
            for (edge_ix, target_ix) in edges.drain(..) {
                let (edge, target) = self.features.index_twice_mut(edge_ix, target_ix);
                let mut new_platforms = platforms;
                if !edge.kinds.is_empty() {
                    for pla_ix in platforms.iter() {
                        if !edge.kinds.iter().any(|k| {
                            k.target.as_ref().map_or(true, |t| {
                                target_spec::eval(&t.to_string(), self.platforms[pla_ix])
                                    .unwrap()
                                    .unwrap()
                            })
                        }) {
                            new_platforms.clear(pla_ix);
                        }
                    }
                }
                *(target.platforms_mut()) = new_platforms;

                if !new_platforms.is_empty() {
                    to_visit.push(target_ix);
                }
            }
        }

        Ok(())
    }

    fn transitive_reduction(&mut self) -> anyhow::Result<()> {
        let graph = &mut self.features;
        let before = graph.edge_count();
        let toposort = petgraph::algo::toposort(&*graph, None)
            .expect("cycling dependencies are not supported");
        let (adj_list, revmap) = petgraph::algo::tred::dag_to_toposorted_adjacency_list::<
            _,
            NodeIndex,
        >(&*graph, &toposort);
        let (reduction, _closure) =
            petgraph::algo::tred::dag_transitive_reduction_closure(&adj_list);

        graph.retain_edges(|x, y| {
            if let Some((f, t)) = x.edge_endpoints(y) {
                reduction.contains_edge(revmap[f.index()], revmap[t.index()])
            } else {
                false
            }
        });
        let after = graph.edge_count();
        debug!("Transitive reduction, edges {before} -> {after}");
        Ok(())
    }

    fn trim_unused_features(&mut self) -> anyhow::Result<()> {
        let mut to_remove = Vec::new();
        loop {
            for f in self.features.externals(petgraph::EdgeDirection::Incoming) {
                if let Feature::External(..) = self.features[f] {
                    to_remove.push(f);
                }
            }
            if to_remove.is_empty() {
                break;
            }
            for f in to_remove.drain(..) {
                self.features.remove_node(f);
            }
        }
        Ok(())
    }

    fn trim_unused_platforms(&mut self) -> anyhow::Result<()> {
        self.features
            .retain_nodes(|g, n| !g[n].platforms().is_empty());
        Ok(())
    }

    fn optimize(&mut self) -> anyhow::Result<()> {
        self.trim_unused_platforms()?;
        self.trim_unused_features()?;
        self.transitive_reduction()?;
        Ok(())
    }

    fn add_package(
        &mut self,
        ix: usize,
        package @ Package {
            dependencies: specified_deps,
            ..
        }: &'a Package,
        Node {
            deps: resolved_deps,
            // features: specified_features,
            ..
        }: &'a Node,
    ) -> anyhow::Result<()> {
        debug!("adding {}", package.id);
        let this_package_pid = Pid(ix, self.meta);
        let base_feature = Fid(this_package_pid, None);
        let base_ix = self.fid_index(base_feature);

        let root_link = Link {
            optional: false,
            kinds: &[],
        };
        if self.workspace_members.contains(&this_package_pid) {
            if package.features.contains_key("default") {
                let default_ix = self.fid_index(Fid(this_package_pid, Some("default")));
                self.features.add_edge(self.root, default_ix, root_link);
            } else {
                self.features.add_edge(self.root, base_ix, root_link);
            }
        }

        // handle dependencies to other packages:
        // optional dependency depends on a local feature with the same name
        // unconditional dependency depends on the base feature
        for resolved_dep in resolved_deps.iter() {
            let specified_dep = match self.library_renames.get(&resolved_dep.pkg) {
                Some(name) => find_dep_by_name(specified_deps, &resolved_dep.name)
                    .or_else(|_| find_dep_by_name(specified_deps, name))?,
                None => find_dep_by_name(specified_deps, &resolved_dep.name)?,
            };

            let dep_pid = *self.cache.get(&resolved_dep.pkg).unwrap();
            let link = Link {
                optional: specified_dep.optional,
                kinds: &resolved_dep.dep_kinds,
            };
            let link_source = if link.optional {
                self.fid_index(Fid(this_package_pid, Some(&resolved_dep.name)))
            } else {
                self.fid_index(Fid(this_package_pid, None))
            };

            if specified_dep.features.is_empty() {
                let base_dep_ix = self.fid_index(Fid(dep_pid, None));
                self.features.add_edge(link_source, base_dep_ix, link);
            } else {
                for feat in specified_dep.features.iter() {
                    let feat_dep_ix = self.fid_index(Fid(dep_pid, Some(feat)));
                    self.features.add_edge(link_source, feat_dep_ix, link);
                }
            }
        }

        // handle local dependencies
        for (local_feat, local_deps) in package.features.iter() {
            let local_ix = self.fid_index(Fid(this_package_pid, Some(local_feat)));

            let local_link = Link {
                optional: false,
                kinds: &[],
            };
            self.features.add_edge(local_ix, base_ix, local_link);

            for other in local_deps.iter() {
                match other.split_once('/') {
                    Some((other_name, other_feat)) => {
                        let dep_declaration = find_dep_by_name(specified_deps, other_name)?;
                        if let Some(dep_resolution) =
                            resolved_deps.iter().find(|d| name_cmp(other_name, &d.name))
                        {
                            let dep_pid = *self.cache.get(&dep_resolution.pkg).unwrap();
                            let link = Link {
                                optional: dep_declaration.optional,
                                kinds: &dep_resolution.dep_kinds,
                            };
                            let local_ix = self.fid_index(Fid(this_package_pid, Some(local_feat)));
                            let other_ix = self.fid_index(Fid(dep_pid, Some(other_feat)));
                            self.features.add_edge(local_ix, other_ix, link);
                        }
                    }
                    None => {
                        let other_ix = self.fid_index(Fid(this_package_pid, Some(other)));
                        self.features.add_edge(local_ix, other_ix, local_link);
                    }
                }
            }
        }

        Ok(())
    }
}

#[derive(Copy, Clone)]
pub struct Pid<'a>(usize, &'a Metadata);

impl Pid<'_> {
    pub fn package(&self) -> &cargo_metadata::Package {
        &self.1.packages[self.0]
    }

    pub fn package_id(&self) -> &cargo_metadata::PackageId {
        &self.1.packages[self.0].id
    }
}

impl<'a> PartialEq for Pid<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<'a> Eq for Pid<'a> {}

impl<'a> PartialOrd for Pid<'a> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

impl<'a> Ord for Pid<'a> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl std::fmt::Debug for Pid<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let meta = &self.1.packages[self.0];
        write!(f, "Pid({} / {})", self.0, meta.id)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Fid<'a>(pub Pid<'a>, pub Option<&'a str>);

#[derive(Debug, Clone, Copy)]
pub struct Link<'a> {
    pub optional: bool,

    pub kinds: &'a [DepKindInfo],
}

impl<'a> Link<'a> {
    pub fn always() -> Self {
        Self {
            optional: false,
            kinds: &[],
        }
    }
}

impl<'a> GraphWalk<'a, NodeIndex, EdgeIndex> for &FeatGraph<'a> {
    fn nodes(&'a self) -> dot::Nodes<'a, NodeIndex> {
        Cow::from(self.features.node_indices().collect::<Vec<_>>())
    }

    fn edges(&'a self) -> dot::Edges<'a, EdgeIndex> {
        Cow::from(self.features.edge_indices().collect::<Vec<_>>())
    }

    fn source(&'a self, edge: &EdgeIndex) -> NodeIndex {
        self.features.edge_endpoints(*edge).unwrap().0
    }

    fn target(&'a self, edge: &EdgeIndex) -> NodeIndex {
        self.features.edge_endpoints(*edge).unwrap().1
    }
}

impl<'a> Labeller<'a, NodeIndex, EdgeIndex> for &FeatGraph<'a> {
    fn graph_id(&'a self) -> dot::Id<'a> {
        dot::Id::new("graphname").unwrap()
    }

    fn node_id(&'a self, n: &NodeIndex) -> dot::Id<'a> {
        dot::Id::new(format!("n{}", n.index())).unwrap()
    }

    fn node_shape(&'a self, _node: &NodeIndex) -> Option<dot::LabelText<'a>> {
        None
    }

    fn node_label(&'a self, n: &NodeIndex) -> dot::LabelText<'a> {
        let mut fmt = String::new();
        match self.features[*n].fid() {
            Some(fid) => {
                let pid = fid.0;
                let graph = pid.1;
                let pkt = &graph.packages[pid.0];
                fmt.push_str(&pkt.name);
                fmt.push_str(&format!(" {}", pkt.version));
                if let Some(feature) = fid.1 {
                    fmt.push('\n');
                    fmt.push_str(feature);
                }

                dot::LabelText::LabelStr(fmt.into())
            }
            None => dot::LabelText::LabelStr("root".into()),
        }
    }

    fn edge_label(&'a self, e: &EdgeIndex) -> dot::LabelText<'a> {
        let _ = e;
        dot::LabelText::LabelStr("".into())
    }

    fn node_style(&'a self, n: &NodeIndex) -> dot::Style {
        if let Some(fid) = self.features[*n].fid() {
            if self.workspace_members.contains(&fid.0) {
                dot::Style::None
            } else {
                dot::Style::Filled
            }
        } else {
            dot::Style::None
        }
    }

    fn node_color(&'a self, _node: &NodeIndex) -> Option<dot::LabelText<'a>> {
        None
    }

    fn edge_end_arrow(&'a self, _e: &EdgeIndex) -> dot::Arrow {
        dot::Arrow::default()
    }

    fn edge_start_arrow(&'a self, _e: &EdgeIndex) -> dot::Arrow {
        dot::Arrow::default()
    }

    fn edge_style(&'a self, _e: &EdgeIndex) -> dot::Style {
        dot::Style::None
    }

    fn edge_color(&'a self, e: &EdgeIndex) -> Option<dot::LabelText<'a>> {
        if self.features[*e].optional {
            Some(dot::LabelText::label("grey"))
        } else {
            Some(dot::LabelText::label("black"))
        }
    }

    fn kind(&self) -> dot::Kind {
        dot::Kind::Digraph
    }
}

fn dump(graph: &FeatGraph) -> anyhow::Result<()> {
    use tempfile::NamedTempFile;
    let mut file = NamedTempFile::new()?;
    dot::render(&graph, &mut file)?;
    std::process::Command::new("xdot")
        .args([file.path()])
        .output()?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Platforms(usize);

impl Platforms {
    pub fn iter(&self) -> PlatformIterator {
        PlatformIterator(self.0)
    }
    //  pub fn iter_mut(&mut self, &[&str]) ->

    pub fn new(count: usize) -> Self {
        assert!(count < 64, "At most 64 unification platforms are supported");
        Platforms((1 << count) - 1)
    }

    pub fn clear(&mut self, ix: usize) {
        self.0 ^= 1 << ix;
    }

    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }
}

pub struct PlatformIterator(usize);

impl Iterator for PlatformIterator {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        if self.0 == 0 {
            return None;
        }
        let lz = self.0.trailing_zeros() as usize;
        self.0 ^= 1 << lz;

        Some(lz as usize)
    }
}
