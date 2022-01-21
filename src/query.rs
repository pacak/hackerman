use guppy::graph::{
    feature::FeatureResolver, DependencyDirection, PackageGraph, PackageMetadata, PackageResolver,
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
pub struct Walker(pub Place);
impl<'g> FeatureResolver<'g> for Walker {
    fn accept(
        &mut self,
        _query: &guppy::graph::feature::FeatureQuery<'g>,
        link: guppy::graph::feature::CrossLink<'g>,
    ) -> bool {
        let t = link.to();

        match self.0 {
            Place::Workspace => {
                // don't leave workspace
                if !t.package().in_workspace() {
                    return false;
                }
            }
            Place::External => {
                // don't go inside of the workspace
                if t.package().in_workspace() {
                    return false;
                }
            }
            _ => {}
        }

        link.normal().is_present() || link.build().is_present()
    }
}

impl<'g> PackageResolver<'g> for Walker {
    fn accept(
        &mut self,
        _query: &guppy::graph::PackageQuery<'g>,
        link: guppy::graph::PackageLink<'g>,
    ) -> bool {
        let t = link.to();
        match self.0 {
            Place::Workspace => {
                // don't leave workspace
                if !t.in_workspace() {
                    return false;
                }
            }
            Place::External => {
                // don't go inside of the workspace
                if t.in_workspace() {
                    return false;
                }
            }
            _ => {}
        }

        link.normal().is_present() || link.build().is_present()
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
