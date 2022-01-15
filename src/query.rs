use guppy::{
    graph::{feature::FeatureResolver, DependencyDirection, PackageResolver},
    DependencyKind,
};

pub enum Place {
    /// Walking in workspace means we never enter external crates
    Workspace,
    /// External walk means we only intersect the workspace
    External,
    /// Ignore workspace boundary
    Both,
}
pub struct Walker(pub DependencyKind, pub Place);
impl<'g> FeatureResolver<'g> for Walker {
    fn accept(
        &mut self,
        query: &guppy::graph::feature::FeatureQuery<'g>,
        link: guppy::graph::feature::CrossLink<'g>,
    ) -> bool {
        let (f, t) = match query.direction() {
            DependencyDirection::Forward => (link.from(), link.to()),
            DependencyDirection::Reverse => (link.to(), link.from()),
        };

        // proc macro follow different rules, skip them here
        if f.package().is_proc_macro() {
            return false;
        }

        match self.1 {
            Place::Workspace => {
                // don't leave workspace
                if !t.package().in_workspace() {
                    return false;
                }
            }
            Place::External => {
                // don't go inside of the workspace
                if f.package().in_workspace() {
                    return false;
                }
            }
            _ => {}
        }

        match self.0 {
            DependencyKind::Normal => link.normal().is_present(),
            DependencyKind::Development => link.dev().is_present() || link.normal().is_present(),
            DependencyKind::Build => link.status_for_kind(self.0).is_present(),
        }
    }
}

impl<'g> PackageResolver<'g> for Walker {
    fn accept(
        &mut self,
        query: &guppy::graph::PackageQuery<'g>,
        link: guppy::graph::PackageLink<'g>,
    ) -> bool {
        let (f, t) = match query.direction() {
            DependencyDirection::Forward => (link.from(), link.to()),
            DependencyDirection::Reverse => (link.to(), link.from()),
        };

        // proc macro follow different rules, skip them here
        if f.is_proc_macro() {
            return false;
        }

        match self.1 {
            Place::Workspace => {
                // don't leave workspace
                if !t.in_workspace() {
                    return false;
                }
            }
            Place::External => {
                // don't go inside of the workspace
                if f.in_workspace() {
                    return false;
                }
            }
            _ => {}
        }

        match self.0 {
            DependencyKind::Normal => link.normal().is_present(),
            DependencyKind::Development => link.dev().is_present() || link.build().is_present(),
            DependencyKind::Build => link.build().is_present(),
        }
    }
}
