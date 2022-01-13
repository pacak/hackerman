use guppy::graph::feature::{CrossLink, FeatureId, FeatureQuery, FeatureResolver};
use guppy::graph::{DependencyDirection, PackageGraph, PackageQuery, PackageResolver};
use guppy::{DependencyKind, PackageId};

pub mod explain;
pub mod hack;
pub mod opts;
pub mod toml;

struct NonMacroKind(DependencyKind);
impl guppy::graph::feature::FeatureResolver<'_> for NonMacroKind {
    fn accept(
        &mut self,
        query: &guppy::graph::feature::FeatureQuery,
        link: guppy::graph::feature::CrossLink,
    ) -> bool {
        link.status_for_kind(self.0).is_present()
            && match query.direction() {
                DependencyDirection::Forward => !link.from().package().is_proc_macro(),
                DependencyDirection::Reverse => !link.to().package().is_proc_macro(),
            }
    }
}

impl PackageResolver<'_> for NonMacroKind {
    fn accept(
        &mut self,
        query: &guppy::graph::PackageQuery,
        link: guppy::graph::PackageLink,
    ) -> bool {
        link.req_for_kind(self.0).is_present()
            && match query.direction() {
                DependencyDirection::Forward => !link.from().is_proc_macro(),
                DependencyDirection::Reverse => !link.to().is_proc_macro(),
            }
    }
}

struct NormalOnly;
impl<'g> FeatureResolver<'g> for NormalOnly {
    fn accept(&mut self, _query: &FeatureQuery<'g>, link: CrossLink<'g>) -> bool {
        link.normal().is_always()
    }
}

impl<'g> PackageResolver<'g> for NormalOnly {
    fn accept(&mut self, _query: &PackageQuery<'g>, link: guppy::graph::PackageLink<'g>) -> bool {
        link.normal().is_present()
    }
}

fn resolve_package<'a>(
    g: &'a PackageGraph,
    name: &'a str,
    mversion: Option<&str>,
) -> anyhow::Result<&'a PackageId> {
    let set = g.resolve_package_name(name);

    match set.len() {
        0 => anyhow::bail!("Package {} is not in use", name),
        1 => {
            let pkg = set.packages(DependencyDirection::Forward).next().unwrap();
            if let Some(version) = mversion {
                if version != pkg.version().to_string() {
                    anyhow::bail!(
                        "Version {} for {} was requested but {} was found instead",
                        version,
                        name,
                        pkg.version()
                    );
                }
            }
            return Ok(pkg.id());
        }
        _ => {
            let version = mversion.ok_or_else(|| {
                let versions = set
                    .root_packages(DependencyDirection::Forward)
                    .map(|p| p.version().to_string())
                    .collect::<Vec<_>>();
                anyhow::anyhow!(
                    "There are multiple versions of {} but no version is specified, {:?}",
                    name,
                    versions
                )
            })?;
            for pkg in set.packages(DependencyDirection::Forward) {
                if pkg.version().to_string() == version {
                    return Ok(pkg.id());
                }
            }

            anyhow::bail!("Package {} {} is not in use", name, version);
        }
    }
}

fn resolve_feature<'a>(
    g: &'a PackageGraph,
    name: &'a str,
    mversion: Option<&str>,
    feature: &'a str,
) -> anyhow::Result<FeatureId<'a>> {
    Ok(FeatureId::new(resolve_package(g, name, mversion)?, feature))
}
