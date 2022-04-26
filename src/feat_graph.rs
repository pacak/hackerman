use crate::metadata::*;
use cargo_metadata::{Metadata, Package, PackageId};
use cargo_platform::Cfg;
use dot::{GraphWalk, Labeller};
use petgraph::graph::{EdgeIndex, NodeIndex};
use petgraph::Graph;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Index;
use tracing::{debug, error, info, trace};

#[derive(Copy, Clone, Ord, PartialEq, Eq, PartialOrd, Debug)]
/// An node for feature graph
///
/// contains information about platforms, package_id and a feature
pub enum Feature<'a> {
    /// "root" node, contains links to all the workspace
    Root,
    /// Fid is a workspace member
    Workspace(Fid<'a>),
    /// Fid is not a workspace member
    External(Fid<'a>),
}

impl std::fmt::Display for Feature<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Feature::Root => f.write_str("root"),
            Feature::Workspace(fid) | Feature::External(fid) => fid.fmt(f),
        }
    }
}

impl<'a> Feature<'a> {
    pub fn fid(&self) -> Option<Fid<'a>> {
        match self {
            Feature::Root => None,
            Feature::Workspace(fid) | Feature::External(fid) => Some(*fid),
        }
    }

    pub fn pid(&self) -> Option<Pid<'a>> {
        self.fid().map(|fid| fid.pid)
    }

    pub fn package_id(&self) -> Option<&PackageId> {
        let Pid(pid, meta) = self.pid()?;
        Some(&meta.packages[pid].id)
    }

    pub fn is_workspace(&self) -> bool {
        match self {
            Feature::Root | Feature::Workspace(_) => true,
            Feature::External(_) => false,
        }
    }

    pub fn is_root(&self) -> bool {
        match self {
            Feature::Root => true,
            Feature::Workspace(_) | Feature::External(_) => false,
        }
    }

    pub fn is_named(&self) -> bool {
        match self {
            Feature::Root => false,
            Feature::Workspace(f) | Feature::External(f) => f.is_named(),
        }
    }
    pub fn is_base(&self) -> bool {
        match self {
            Feature::Root => false,
            Feature::Workspace(f) | Feature::External(f) => !f.is_named(),
        }
    }

    pub fn get_external(&self) -> Option<Fid> {
        match self {
            Feature::External(f) => Some(*f),
            Feature::Root | Feature::Workspace(_) => None,
        }
    }
}

pub struct FeatGraph<'a> {
    /// root node, should be 0
    pub root: NodeIndex,
    /// set of workspace members
    pub workspace_members: BTreeSet<Pid<'a>>,
    /// a dependency graph between features
    /// Feature = Fid + decoration if it's external, internal or root
    pub features: Graph<Feature<'a>, Link>,
    /// A way to look up fids in features
    fids: BTreeMap<Fid<'a>, NodeIndex>,
    /// a lookup cache from cargo metadata's PackageId to hackerman's Pid
    cache: BTreeMap<&'a PackageId, Pid<'a>>,

    pub fid_cache: BTreeMap<Fid<'a>, NodeIndex>,

    /// cargo metadata
    meta: &'a Metadata,

    pub platforms: Vec<&'a str>,
    pub cfgs: Vec<Cfg>,
    pub triggers: BTreeMap<Pid<'a>, Vec<Trigger<'a>>>,

    pub focus_nodes: Option<BTreeSet<NodeIndex>>,
    pub focus_edges: Option<BTreeSet<EdgeIndex>>,
    pub focus_targets: Option<BTreeSet<NodeIndex>>,
}

impl<'a> Index<Pid<'a>> for FeatGraph<'a> {
    type Output = NodeIndex;

    fn index(&self, index: Pid<'a>) -> &Self::Output {
        &self.fid_cache[&index.root()]
    }
}

#[derive(Debug)]
pub struct Trigger<'a> {
    // foo.toml:
    // [features]
    // serde1 = ["dep:serde", "rgb?/serde"]
    // when both `feature` and `weak_dep` are present we must include `to_add`
    pub package: Pid<'a>,   // foo
    pub feature: Fid<'a>,   // serde1
    pub weak_dep: Pid<'a>,  // rgb
    pub weak_feat: Fid<'a>, // rgb/serde
}

impl<'a> FeatGraph<'a> {
    pub fn fid_index(&mut self, fid: Fid<'a>) -> NodeIndex {
        *self.fids.entry(fid).or_insert_with(|| {
            if self.workspace_members.contains(&fid.pid) {
                self.features.add_node(Feature::Workspace(fid))
            } else {
                self.features.add_node(Feature::External(fid))
            }
        })
    }

    pub fn init(
        meta: &'a Metadata,
        platforms: Vec<&'a str>,
        cfgs: Vec<Cfg>,
    ) -> anyhow::Result<Self> {
        if meta.resolve.is_none() {
            anyhow::bail!("Cargo couldn't produce resolved dependencies")
        }

        let cache = meta
            .packages
            .iter()
            .enumerate()
            .map(|(ix, package)| (&package.id, Pid(ix, meta)))
            .collect::<BTreeMap<_, _>>();

        let mut features = Graph::new();
        let root = features.add_node(Feature::Root);

        let mut graph = Self {
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
            triggers: BTreeMap::new(),
            fid_cache: BTreeMap::new(),
            cache,
            meta,
            cfgs,
            focus_nodes: None,
            focus_edges: None,
            focus_targets: None,
        };

        for (ix, package) in meta.packages.iter().enumerate() {
            graph.add_package(ix, package, &meta.packages)?;
        }

        graph.rebuild_cache()?;

        Ok(graph)
    }

    pub fn optimize(&mut self) -> anyhow::Result<()> {
        info!("Optimization pass: trim unused features");
        self.trim_unused_features()?;

        info!("Optimization pass: transitive reduction");
        self.transitive_reduction()?;

        self.rebuild_cache()?;
        Ok(())
    }

    pub fn rebuild_cache(&mut self) -> anyhow::Result<()> {
        info!("Rebuilding feature id cache");
        self.fids.clear();
        for node in self.features.node_indices() {
            if let Some(fid) = self.features[node].fid() {
                self.fids.insert(fid, node);
            }

            if let Some(feat) = self.features[node].fid() {
                self.fid_cache.insert(feat, node);
            }
        }
        Ok(())
    }

    fn transitive_reduction(&mut self) -> anyhow::Result<()> {
        let graph = &mut self.features;
        let before = graph.edge_count();
        let toposort = match petgraph::algo::toposort(&*graph, None) {
            Ok(t) => t,
            Err(err) => {
                error!("Cyclic dependencies are detected {err:?}, skipping transitive reduction");
                return Ok(());
            }
        };

        use petgraph::algo::tred::dag_to_toposorted_adjacency_list;
        let (adj_list, revmap) =
            dag_to_toposorted_adjacency_list::<_, NodeIndex>(&*graph, &toposort);
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

    pub fn resolve_package_index(&self, ix: PackageIndex) -> Pid<'a> {
        let package = &self.meta.packages[ix.0];
        *self.cache.get(&package.id).unwrap()
    }

    fn add_package(
        &mut self,
        ix: usize,
        package: &'a Package,
        packages: &'a [Package],
    ) -> anyhow::Result<()> {
        debug!("== adding package {}", package.id);
        let this = Pid(ix, self.meta);
        let base_ix = self.fid_index(this.base());

        let workspace_member = self.workspace_members.contains(&this);

        // root contains links to all the workspace members
        if workspace_member {
            self.add_edge(self.root, this, false, DepKindInfo::NORMAL)?;
        }

        // resolve and cache crate dependencies and create a cache mapping name to dep
        let mut deps = BTreeMap::new();
        for dep in &package.dependencies {
            if !workspace_member && dep.kind == cargo_metadata::DependencyKind::Development {
                trace!("Skipping external dev dependency {dep:?}");
                continue;
            }

            // get resolved package - should be there in at most one matching copy...
            let resolved = match packages
                .iter()
                .find(|p| p.name == dep.name && dep.req.matches(&p.version))
            {
                Some(res) => res,
                None => {
                    debug!(
                        "cargo metadta did not include optional dependency \"{} {}\" \
                        requested by \"{} {}\", skipping",
                        dep.name, dep.req, package.name, package.version
                    );
                    continue;
                }
            };

            // feature dependencies:
            //
            // - optional dependencies are linked from named feature
            // - requred dependenceis are linked fromb base
            let this = if dep.optional {
                this.named(&dep.name).get_index(self)?
            } else {
                base_ix
            };

            let remote;

            //  dependencies that have default target are linked to that target
            //  otherwise dependencies are linked to
            if dep.uses_default_features {
                remote = Some(self.add_edge(this, resolved, false, dep.into())?);
            } else if let Some(pid) = self.cache.get(&resolved.id) {
                let fid = pid.base();
                remote = Some(self.add_edge(this, fid, false, dep.into())?);
            } else {
                remote = None;
            }
            // if additional features on dependency are required - we add them all
            for feat in &dep.features {
                self.add_edge(this, (resolved, feat.as_str()), false, dep.into())?;
            }

            if let Some(remote) = remote {
                deps.insert(
                    dep.rename
                        .as_ref()
                        .map_or_else(|| resolved.name.clone(), |r| r.clone()),
                    (resolved, dep, remote),
                );
            }
        }

        for (this_feat, feat_deps) in package.features.iter() {
            let feat_ix = self.fid_index(this.named(this_feat));
            self.add_edge(feat_ix, base_ix, false, DepKindInfo::NORMAL)?;

            for feat_dep in feat_deps.iter() {
                match FeatTarget::from(feat_dep.as_str()) {
                    FeatTarget::Named { name } => {
                        self.add_edge(feat_ix, this.named(name), false, DepKindInfo::NORMAL)?;
                    }
                    FeatTarget::Dependency { krate } => {
                        if let Some(&(_dep, link, remote)) = deps.get(krate) {
                            self.add_edge(feat_ix, remote, true, link.into())?;
                        } else {
                            debug!("skipping disabled optional dependency {krate}");
                        }
                    }
                    FeatTarget::Remote { krate, feat } => {
                        if let Some(&(dep, link, _remote)) = deps.get(krate) {
                            self.add_edge(feat_ix, (dep, feat), true, link.into())?;
                        } else {
                            debug!("skipping disabled optional dependency {krate}");
                        }
                    }
                    FeatTarget::Cond { krate, feat } => {
                        if let Some(dep) = deps
                            .get(krate)
                            .and_then(|&(dep, _link, _remote)| self.cache.get(&dep.id).copied())
                        {
                            let trigger = Trigger {
                                package: this,
                                feature: this.named(this_feat),
                                weak_dep: dep,
                                weak_feat: dep.named(feat),
                            };
                            self.triggers
                                .entry(this)
                                .or_insert_with(Vec::new)
                                .push(trigger);
                        } else {
                            debug!("skipping disabled optional dependency {krate}");
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub fn add_edge<A, B>(
        &mut self,
        a: A,
        b: B,
        optional: bool,
        kind: DepKindInfo,
    ) -> anyhow::Result<NodeIndex>
    where
        A: HasIndex<'a>,
        B: HasIndex<'a>,
    {
        let a = a.get_index(self)?;
        let b = b.get_index(self)?;
        trace!(
            "adding {}edge {a:?} -> {b:?}: {kind:?}\n\t{:?}\n\t{:?}",
            if optional { "optional " } else { "" },
            self.features[a],
            self.features[b]
        );

        if let Some(index) = self.features.find_edge(a, b) {
            let old_link = &mut self.features[index];
            if !old_link.kinds.contains(&kind) {
                old_link.kinds.push(kind);
            }
            old_link.optional &= optional;
        } else {
            let link = Link {
                optional,
                kinds: vec![kind],
            };
            self.features.add_edge(a, b, link);
        }
        Ok(b)
    }

    pub fn packages_by_name(&self, name: &str) -> Vec<Pid> {
        self.meta
            .packages
            .iter()
            .filter(|p| p.name == name)
            .filter_map(|p| self.cache.get(&p.id))
            .copied()
            .collect::<Vec<_>>()
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

    pub fn package_index(&self) -> PackageIndex {
        PackageIndex(self.0)
    }
}

// refers to a single package
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Debug)]
pub struct PackageIndex(usize);

// refers to a single feature
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Debug)]
pub struct FeatureIndex(NodeIndex);

impl<'a> Pid<'a> {
    pub fn root(self) -> Fid<'a> {
        if self.package().features.contains_key("default") {
            self.named("default")
        } else {
            self.base()
        }
    }

    pub fn base(self) -> Fid<'a> {
        Fid {
            pid: self,
            dep: Feat::Base,
        }
    }
    pub fn named(self, name: &'a str) -> Fid<'a> {
        Fid {
            pid: self,
            dep: Feat::Named(name),
        }
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
pub struct Fid<'a> {
    /// this feature originates from
    pub pid: Pid<'a>,
    pub dep: Feat<'a>,
}

impl std::fmt::Display for Fid<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let id = self.pid.package_id();
        match self.dep {
            Feat::Base => write!(f, "{id}"),
            Feat::Named(name) => write!(f, "{id}:{name}"),
        }
    }
}

impl std::fmt::Display for Feat<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Feat::Base => f.write_str(":base:"),
            Feat::Named(name) => f.write_str(name),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Feat<'a> {
    /// Base package itself
    Base,
    /// internally defined named feature
    Named(&'a str),
}

impl<'a> GraphWalk<'a, NodeIndex, EdgeIndex> for FeatGraph<'a> {
    fn nodes(&'a self) -> dot::Nodes<'a, NodeIndex> {
        Cow::from(match &self.focus_nodes {
            Some(f) => f.iter().copied().collect::<Vec<_>>(),
            None => self.features.node_indices().collect::<Vec<_>>(),
        })
    }

    fn edges(&'a self) -> dot::Edges<'a, EdgeIndex> {
        Cow::from(match &self.focus_edges {
            Some(f) => f.iter().copied().collect::<Vec<_>>(),
            None => self.features.edge_indices().collect::<Vec<_>>(),
        })
    }

    fn source(&'a self, edge: &EdgeIndex) -> NodeIndex {
        self.features.edge_endpoints(*edge).unwrap().0
    }

    fn target(&'a self, edge: &EdgeIndex) -> NodeIndex {
        self.features.edge_endpoints(*edge).unwrap().1
    }
}

impl<'a> Labeller<'a, NodeIndex, EdgeIndex> for FeatGraph<'a> {
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
                let pid = fid.pid;
                let graph = pid.1;
                let pkt = &graph.packages[pid.0];
                fmt.push_str(&pkt.name);
                fmt.push_str(&format!(" {}", pkt.version));
                match fid.dep {
                    Feat::Base => {}
                    Feat::Named(name) => {
                        fmt.push('\n');
                        fmt.push_str(name);
                    }
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
            if self.workspace_members.contains(&fid.pid) {
                dot::Style::None
            } else {
                dot::Style::Filled
            }
        } else {
            dot::Style::None
        }
    }

    fn node_color(&'a self, node: &NodeIndex) -> Option<dot::LabelText<'a>> {
        self.focus_targets
            .as_ref()?
            .contains(node)
            .then(|| dot::LabelText::LabelStr("pink".into()))
    }

    fn edge_end_arrow(&'a self, _e: &EdgeIndex) -> dot::Arrow {
        dot::Arrow::default()
    }

    fn edge_start_arrow(&'a self, _e: &EdgeIndex) -> dot::Arrow {
        dot::Arrow::default()
    }

    fn edge_style(&'a self, e: &EdgeIndex) -> dot::Style {
        if self.features[*e].is_dev_only() {
            dot::Style::Dashed
        } else {
            dot::Style::None
        }
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

pub fn dump(graph: &FeatGraph) -> anyhow::Result<()> {
    use tempfile::NamedTempFile;
    let mut file = NamedTempFile::new()?;
    dot::render(graph, &mut file)?;
    std::process::Command::new("xdot")
        .args([file.path()])
        .output()?;
    Ok(())
}

pub trait HasIndex<'a> {
    fn get_index(self, graph: &mut FeatGraph<'a>) -> anyhow::Result<NodeIndex>;
}

impl HasIndex<'_> for NodeIndex {
    fn get_index(self, _graph: &mut FeatGraph) -> anyhow::Result<NodeIndex> {
        Ok(self)
    }
}

impl<'a> HasIndex<'a> for Fid<'a> {
    fn get_index(self, graph: &mut FeatGraph<'a>) -> anyhow::Result<NodeIndex> {
        Ok(graph.fid_index(self))
    }
}

impl<'a> HasIndex<'a> for Pid<'a> {
    fn get_index(self, graph: &mut FeatGraph<'a>) -> anyhow::Result<NodeIndex> {
        if self.package().features.contains_key("default") {
            Ok(graph.fid_index(self.named("default")))
        } else {
            Ok(graph.fid_index(self.base()))
        }
    }
}

impl<'a> HasIndex<'a> for &'a Package {
    fn get_index(self, graph: &mut FeatGraph<'a>) -> anyhow::Result<NodeIndex> {
        (*graph
            .cache
            .get(&self.id)
            .ok_or_else(|| anyhow::anyhow!("No cached value for {:?}", self.id))?)
        .get_index(graph)
    }
}

impl<'a> HasIndex<'a> for (&'a Package, &'a str) {
    fn get_index(self, graph: &mut FeatGraph<'a>) -> anyhow::Result<NodeIndex> {
        let package_id = &self.0.id;
        let feat = self.1;
        let pid = *graph
            .cache
            .get(package_id)
            .ok_or_else(|| anyhow::anyhow!("No cached value for {package_id:?}"))?;
        let fid = pid.named(feat);
        fid.get_index(graph)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Platforms(usize);

impl Platforms {
    pub fn iter(&self) -> PlatformIterator {
        PlatformIterator(self.0)
    }

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

impl std::ops::BitOrAssign for Platforms {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0
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

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FeatTarget<'a> {
    Named { name: &'a str },
    Dependency { krate: &'a str },
    Remote { krate: &'a str, feat: &'a str },
    Cond { krate: &'a str, feat: &'a str },
}

impl<'a> From<&'a str> for FeatTarget<'a> {
    fn from(s: &'a str) -> Self {
        if let Some(krate) = s.strip_prefix("dep:") {
            FeatTarget::Dependency { krate }
        } else if let Some((krate, feat)) = s.split_once("?/") {
            FeatTarget::Cond { krate, feat }
        } else if let Some((krate, feat)) = s.split_once('/') {
            FeatTarget::Remote { krate, feat }
        } else {
            FeatTarget::Named { name: s }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn feat_target() {
        use FeatTarget::*;
        assert_eq!(FeatTarget::from("quote"), Named { name: "quote" });
        assert_eq!(
            FeatTarget::from("dep:serde_json"),
            Dependency {
                krate: "serde_json"
            }
        );
        assert_eq!(
            FeatTarget::from("syn/extra-tr"),
            Remote {
                krate: "syn",
                feat: "extra-tr"
            }
        );
        assert_eq!(
            FeatTarget::from("rgb?/serde"),
            Cond {
                krate: "rgb",
                feat: "serde"
            }
        );
    }

    fn get_demo_meta(ix: usize) -> anyhow::Result<Metadata> {
        let path = format!("{}/demo/{ix}/metadata.json", env!("CARGO_MANIFEST_DIR"));
        let data = std::fs::read_to_string(path)?;
        Ok(cargo_metadata::MetadataCommand::parse(data)?)
    }

    fn process_fg_with<F>(ix: usize, op: F) -> anyhow::Result<()>
    where
        F: FnOnce(&mut FeatGraph) -> anyhow::Result<()>,
    {
        let meta = get_demo_meta(ix)?;
        let platform = target_spec::Platform::current()?;
        let triplets = vec![platform.triple_str()];
        let mut fg = FeatGraph::init(&meta, triplets, Vec::new())?;
        op(&mut fg)
    }

    #[test]
    fn metadata_snapshot_1() -> anyhow::Result<()> {
        process_fg_with(1, |_| Ok(()))?;
        Ok(())
    }

    #[test]
    fn metadata_snapshot_2() -> anyhow::Result<()> {
        process_fg_with(2, |_| Ok(()))?;
        Ok(())
    }

    #[test]
    fn metadata_snapshot_3() -> anyhow::Result<()> {
        process_fg_with(3, |_| Ok(()))?;
        Ok(())
    }

    #[test]
    fn metadata_snapshot_4() -> anyhow::Result<()> {
        process_fg_with(4, |_| Ok(()))?;
        Ok(())
    }

    #[test]
    fn metadata_snapshot_5() -> anyhow::Result<()> {
        process_fg_with(5, |fg| {
            dump(fg)?;

            Ok(())
        })
    }
}
impl Fid<'_> {
    fn is_named(&self) -> bool {
        match self.dep {
            Feat::Base => false,
            Feat::Named(_) => true,
        }
    }

    pub fn base(&self) -> Self {
        Self {
            dep: Feat::Base,
            ..*self
        }
    }
}
