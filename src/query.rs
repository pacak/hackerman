use guppy::{
    graph::{
        feature::FeatureResolver, DependencyDirection, PackageGraph, PackageMetadata,
        PackageResolver,
    },
    DependencyKind,
};

#[derive(Copy, Clone)]
pub enum Place {
    /// Walking in workspace means we never enter external crates
    Workspace,
    /// External walk means we only intersect the workspace
    External,
    /// Ignore workspace boundary
    Both,
}
#[derive(Copy, Clone)]
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

pub fn packages_by_name_and_version<'a>(
    package_graph: &'a PackageGraph,
    name: &'a str,
    version: Option<&'a str>,
) -> anyhow::Result<Vec<PackageMetadata<'a>>> {
    let mut packages = package_graph
        .resolve_package_name(name)
        .packages(DependencyDirection::Forward)
        .collect::<Vec<_>>();
    let present = !packages.is_empty();
    if let Some(version) = version {
        packages.retain(|p| p.version().to_string() == version);
        if present && packages.is_empty() {
            anyhow::bail!("Package {} v{} is not in use", name, version);
        }
    }
    if packages.is_empty() {
        anyhow::bail!("Package {} is not in use", name)
    }
    Ok(packages)
}
