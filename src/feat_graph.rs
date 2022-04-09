use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use cargo_metadata::{DepKindInfo, Dependency, Metadata, PackageId};
use dot::{GraphWalk, Labeller};
use petgraph::{
    graph::{EdgeIndex, NodeIndex},
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

fn root() -> NodeIndex {
    NodeIndex::new(0)
}

#[derive(Copy, Clone, Ord, PartialEq, Eq, PartialOrd)]
pub enum Feature<'a> {
    Root,
    Workspace(Fid<'a>),
    External(Fid<'a>),
}

impl<'a> Feature<'a> {
    pub fn fid(&self) -> Option<Fid<'a>> {
        match self {
            Feature::Root => None,
            Feature::Workspace(fid) | Feature::External(fid) => Some(*fid),
        }
    }
}

pub struct FeatGraph2<'a> {
    pub workspace_members: BTreeSet<Pid<'a>>,
    pub features: Graph<Feature<'a>, Link<'a>>,
    pub fids: BTreeMap<Fid<'a>, NodeIndex>,
    pub pids: BTreeMap<Pid<'a>, NodeIndex>,
    pub cache: BTreeMap<&'a PackageId, Pid<'a>>,
}

fn find_dep_by_pid<'a>(deps: &'a [Dependency], pid: Pid<'a>) -> anyhow::Result<&'a Dependency> {
    let name = &pid.package().name;
    // there are some very strange ideas about what is a valid crate is name and how to compare
    // them out there
    fn cmp(a: &str, b: &str) -> bool {
        a.chars().zip(b.chars()).all(|(l, r)| {
            l.to_ascii_lowercase() == r.to_ascii_lowercase() || (l == '-' && r == '_')
        })
    }

    deps.iter()
        .find(|d| cmp(&d.name, name) || d.rename.as_ref().map_or(false, |r| r == name))
        .ok_or_else(|| anyhow::anyhow!("No dependency named {name}"))
}

fn find_dep_by_name<'a>(deps: &'a [Dependency], name: &'a str) -> anyhow::Result<&'a Dependency> {
    // there are some very strange ideas about what is a valid crate is name and how to compare
    // them out there
    fn cmp(a: &str, b: &str) -> bool {
        a.chars().zip(b.chars()).all(|(l, r)| {
            l.to_ascii_lowercase() == r.to_ascii_lowercase() || (l == '-' && r == '_')
        })
    }

    deps.iter()
        .find(|d| cmp(&d.name, name) || d.rename.as_ref().map_or(false, |r| r == name))
        .ok_or_else(|| anyhow::anyhow!("No dependency named {name}"))
}

impl<'a> FeatGraph2<'a> {
    pub fn fid_index(&mut self, fid: Fid<'a>) -> NodeIndex {
        *self.fids.entry(fid).or_insert_with(|| {
            if self.workspace_members.contains(&fid.0) {
                self.features.add_node(Feature::Workspace(fid))
            } else {
                self.features.add_node(Feature::External(fid))
            }
        })
    }

    pub fn init(meta: &'a Metadata) -> anyhow::Result<Self> {
        // resolves for easier access
        let resolves = &meta
            .resolve
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("cargo metadata couldn't resolve the depdendencies"))?
            .nodes;

        // fast mapping from cargo's PackageId to Pid
        let cache = meta
            .packages
            .iter()
            .enumerate()
            .map(|(ix, package)| (&package.id, Pid(ix, meta)))
            .collect::<BTreeMap<_, _>>();

        let mut f = Self {
            workspace_members: meta
                .workspace_members
                .iter()
                .flat_map(|pid| cache.get(pid))
                .copied()
                .collect::<BTreeSet<_>>(),
            features: Graph::new(),
            fids: BTreeMap::new(),
            pids: BTreeMap::new(),
            cache,
        };

        for (ix, (package, deps)) in meta.packages.iter().zip(resolves.iter()).enumerate() {
            assert_eq!(
                package.id, deps.id,
                "packages and resolves should come in the same order from cargo metadata"
            );

            println!("package: {:?}", package);
            println!("resolves: {:?}", deps);

            let pid = Pid(ix, meta);
            let fid = Fid(pid, None);

            // handle dependencies for other packages
            for dep in deps.deps.iter() {
                let dep_pid = *f.cache.get(&dep.pkg).unwrap();

                let dep_declaration = find_dep_by_pid(&package.dependencies, dep_pid)?;
                let link = Link {
                    optional: dep_declaration.optional,
                    kinds: &dep.dep_kinds,
                };
                let link_source = if link.optional {
                    f.fid_index(Fid(pid, Some(&dep.name)))
                } else {
                    f.fid_index(Fid(pid, None))
                };

                let base_dep_ix = f.fid_index(Fid(dep_pid, None));
                f.features.add_edge(link_source, base_dep_ix, link);

                for feat in dep_declaration.features.iter() {
                    let feat_dep_ix = f.fid_index(Fid(dep_pid, Some(feat)));
                    f.features.add_edge(link_source, feat_dep_ix, link);
                }
            }

            let base_ix = f.fid_index(fid);
            for (local_feat, local_deps) in package.features.iter() {
                let local_ix = f.fid_index(Fid(pid, Some(local_feat)));

                let local_link = Link {
                    optional: false,
                    kinds: &[],
                };
                f.features.add_edge(local_ix, base_ix, local_link);
                /*
                                for other in local_deps.iter() {
                                    match other.split_once('/') {
                                        Some((other_name, other_feat)) => {
                                            let dep_declaration = find_dep_by_name(&package.dependencies, other_name)?;
                                            let dep_pkg = deps.deps.iter().find(|d| d.name == dep_declaration.name).unwrap().pkg;
                                            let dep_pid = f.cache.get(&dep_pkg).unwrap();
                                            let link = Link {
                                                optional: dep_declaration.optional,
                                                kinds: &dep_declaration.dep_kinds,
                                            };
                                            let other_ix = f.fid_index(dep_pid)
                                        } // todo!("{:?} {:?}", a, b),
                                        None => {
                                            let other_ix = f.fid_index(Fid(pid, Some(other)));
                                            f.features.add_edge(local_ix, other_ix, local_link);
                                        }
                                    }
                                }
                */
                //                println!("{local_feat:?}\n{local_deps:?}")
            }

            /*
                        let feats = std::iter::once(Fid(pid, None))
                            .chain(
                                package
                                    .dependencies
                                    .iter()
                                    .filter(|d| d.optional)
                                    .map(|d| Fid(pid, Some(&d.name))),
                            )
                            .chain(package.features.keys().map(|n| Fid(pid, Some(n))));

                        println!("pid: {:?}", pid);

                        //            for l in links.iter() {
                        //                println!("\t{:?}", l);
                        //            }

                        for fid in feats {
                            println!("\tfid: {:?}", fid);
                        }
                        for dep in package.dependencies.iter() {
                            println!(
                                "\t{:?}: opt: {:?}, def: {:?} {:?}",
                                dep.kind, dep.optional, dep.uses_default_features, dep
                            );
                        }

                        for dep in deps.deps.iter() {
                            println!("\t,{:?}", dep);
                        }
            */
            println!("\n\n");
        }

        Ok(f)
    }
}

#[derive(Copy, Clone)]
pub struct Pid<'a>(usize, &'a Metadata);

impl Pid<'_> {
    pub fn package(&self) -> &cargo_metadata::Package {
        &self.1.packages[self.0]
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
pub struct Fid<'a>(Pid<'a>, Option<&'a str>);

#[derive(Debug, Clone, Copy)]
pub struct Link<'a> {
    pub optional: bool,

    pub kinds: &'a [DepKindInfo],
}

impl<'a> GraphWalk<'a, NodeIndex, EdgeIndex> for &FeatGraph2<'a> {
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

impl<'a> Labeller<'a, NodeIndex, EdgeIndex> for &FeatGraph2<'a> {
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
        let fid = self.features[*n];
        let pid = fid.fid().unwrap().0;
        let graph = pid.1;
        let pkt = &graph.packages[pid.0];
        fmt.push_str(&pkt.name);
        if let Some(feature) = fid.fid().unwrap().1 {
            fmt.push('\n');
            fmt.push_str(feature);
        }

        dot::LabelText::LabelStr(fmt.into())
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
