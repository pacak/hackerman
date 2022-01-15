use guppy::graph::{feature::CrossLink, PackageMetadata};
use guppy::PackageId;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

pub struct FeatDepGraph<'a> {
    pub roots: BTreeSet<&'a PackageId>,
    pub nodes: BTreeMap<&'a PackageId, PackageMetadata<'a>>,
    pub features: BTreeMap<&'a PackageId, BTreeSet<&'a str>>,
    pub edges: BTreeMap<(&'a PackageId, &'a PackageId), Vec<CrossLink<'a>>>,
}

type Link<'a> = Vec<CrossLink<'a>>;
impl<'a> dot::GraphWalk<'a, PackageMetadata<'a>, Link<'a>> for FeatDepGraph<'a> {
    fn nodes(&'a self) -> Cow<'a, [PackageMetadata<'a>]> {
        Cow::from(self.nodes.values().cloned().collect::<Vec<_>>())
    }

    fn edges(&'a self) -> std::borrow::Cow<'a, [Vec<CrossLink<'a>>]> {
        Cow::from(self.edges.values().cloned().collect::<Vec<_>>())
    }

    fn source(&'a self, edge: &Vec<CrossLink<'a>>) -> PackageMetadata<'a> {
        edge.first().unwrap().package_link().from()
    }

    fn target(&'a self, edge: &Vec<CrossLink<'a>>) -> PackageMetadata<'a> {
        edge.first().unwrap().package_link().to()
    }
}

impl<'a> dot::Labeller<'a, PackageMetadata<'a>, Link<'a>> for FeatDepGraph<'a> {
    fn node_shape(&'a self, _node: &PackageMetadata<'a>) -> Option<dot::LabelText<'a>> {
        None
    }

    fn node_label(&'a self, node: &PackageMetadata<'a>) -> dot::LabelText<'a> {
        let mut fmt = String::new();
        fmt.push_str(node.name());
        if !node.in_workspace() {
            fmt.push('\n');
            fmt.push_str(&node.version().to_string());
        }

        if let Some(f) = self.features.get(node.id()) {
            fmt.push('\n');
            fmt.push_str(&format!("{f:?}"))
        }
        dot::LabelText::label(fmt)
    }

    fn edge_label(&'a self, link: &Link<'a>) -> dot::LabelText<'a> {
        let xs = link
            .iter()
            .flat_map(|l| l.to().feature_id().feature())
            .filter(|x| x != &"default")
            .collect::<Vec<_>>();
        if xs.is_empty() {
            dot::LabelText::LabelStr("".into())
        } else {
            dot::LabelText::LabelStr(format!("{xs:?}").into())
        }
    }

    fn node_style(&'a self, _n: &PackageMetadata<'a>) -> dot::Style {
        dot::Style::None
    }

    fn node_color(&'a self, node: &PackageMetadata<'a>) -> Option<dot::LabelText<'a>> {
        if node.in_workspace() {
            Some(dot::LabelText::label("red"))
        } else if self.roots.contains(node.id()) {
            Some(dot::LabelText::label("green"))
        } else {
            None
        }
    }

    fn edge_end_arrow(&'a self, _e: &Link<'a>) -> dot::Arrow {
        dot::Arrow::default()
    }

    fn edge_start_arrow(&'a self, _e: &Link<'a>) -> dot::Arrow {
        dot::Arrow::default()
    }

    fn edge_style(&'a self, e: &Link<'a>) -> dot::Style {
        let devs = e.iter().filter(|l| l.dev_only()).count();
        if e.len() == devs {
            dot::Style::Dotted
        } else if devs == 0 {
            dot::Style::Solid
        } else {
            dot::Style::Dashed
        }
    }

    fn edge_color(&'a self, _e: &Link<'a>) -> Option<dot::LabelText<'a>> {
        None
    }

    fn kind(&self) -> dot::Kind {
        dot::Kind::Digraph
    }

    fn graph_id(&'a self) -> dot::Id<'a> {
        dot::Id::new("features").unwrap()
    }

    fn node_id(&'a self, n: &PackageMetadata<'a>) -> dot::Id<'a> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        n.id().hash(&mut hasher);
        let x = hasher.finish();

        dot::Id::new(format!("n{}", x)).unwrap()
    }
}
