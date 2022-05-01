use crate::{
    feat_graph::{FeatTarget, Pid},
    hack::Ty,
};
use cargo_metadata::{camino::Utf8PathBuf, Version};
use std::collections::{BTreeSet, HashMap};
use tracing::debug;

fn optimize_feats(declared: &HashMap<String, Vec<String>>, requested: &mut BTreeSet<String>) {
    let mut implicit = BTreeSet::new();
    for req in requested.iter() {
        for dep in declared.get(req).iter().flat_map(|x| x.iter()) {
            if let FeatTarget::Named { name } = FeatTarget::from(dep.as_str()) {
                implicit.insert(name);
            }
        }
    }
    for imp in &implicit {
        requested.remove(*imp);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use super::{optimize_feats, PackageSource};
    fn check(req: &[&str], decl: &[(&str, &[&str])], exp: &[&str]) {
        let mut requested = req
            .iter()
            .copied()
            .map(String::from)
            .collect::<BTreeSet<_>>();

        let mut declared = HashMap::new();
        for (key, vals) in decl.iter() {
            declared.insert(
                key.to_string(),
                vals.iter().copied().map(String::from).collect::<Vec<_>>(),
            );
        }
        optimize_feats(&declared, &mut requested);
        let expected = exp
            .iter()
            .copied()
            .map(String::from)
            .collect::<BTreeSet<_>>();
        assert_eq!(requested, expected);
    }

    #[test]
    fn optimize_feats_1() {
        check(&["one", "default"], &[("default", &["one"])], &["default"]);
    }

    #[test]
    fn optimize_feats_2() {
        check(
            &["one", "default"],
            &[("default", &["two"])],
            &["default", "one"],
        );
    }

    #[test]
    fn optimize_feats_3() {
        check(
            &["one", "two", "default"],
            &[("default", &["one", "two"])],
            &["default"],
        );
    }

    const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";
    const GIT_0: &str = "git+https://github.com/rust-lang/cargo.git?branch=main#0227f048";
    const GIT_1: &str = "git+https://github.com/rust-lang/cargo.git?tag=v0.46.0#0227f048";
    const GIT_2: &str = "git+https://github.com/rust-lang/cargo.git?rev=0227f048#0227f048";
    const GIT_3: &str = "git+https://github.com/gyscos/zstd-rs.git#bc874a57";

    #[test]
    fn parse_sources() -> anyhow::Result<()> {
        PackageSource::try_from(CRATES_IO)?;
        PackageSource::try_from(GIT_0)?;
        PackageSource::try_from(GIT_1)?;
        PackageSource::try_from(GIT_2)?;
        PackageSource::try_from(GIT_3)?;
        Ok(())
    }
}

impl<'a> TryFrom<&'a str> for PackageSource<'a> {
    type Error = anyhow::Error;
    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        if let Some(registry) = value.strip_prefix("registry+") {
            Ok(PackageSource::Registry(registry))
        } else if let Some(repo) = value.strip_prefix("git+") {
            if let Some((url, _)) = repo.split_once('#') {
                Ok(PackageSource::Git(url))
            } else {
                Ok(PackageSource::Git(repo))
            }
        } else {
            anyhow::bail!("Not sure what package source is {value}");
        }
    }
}

impl<'a> ChangePackage<'a> {
    #[allow(clippy::similar_names)]
    pub fn make(
        importer: Pid<'a>,
        importee: Pid<'a>,
        ty: Ty,
        rename: bool,
        mut feats: BTreeSet<String>,
    ) -> Self {
        let package = importee.package();
        optimize_feats(&package.features, &mut feats);
        if let Some(src) = &package.source {
            let source = PackageSource::try_from(src.repr.as_str()).unwrap();
            ChangePackage {
                name: package.name.clone(),
                ty,
                version: package.version.clone(),
                source,
                feats,
                rename,
            }
        } else {
            let source = match relative_import_dir(importer, importee) {
                Some(path) => PackageSource::File { path },
                None => {
                    let manifest = &importee.package().manifest_path;
                    debug!(
                        "Using absolute manifest path for {:?}: {}",
                        importee, manifest
                    );
                    PackageSource::File {
                        path: manifest
                            .parent()
                            .expect("Very strange manifest path")
                            .to_path_buf(),
                    }
                }
            };
            ChangePackage {
                name: package.name.clone(),
                ty,
                version: package.version.clone(),
                source,
                feats,
                rename,
            }
        }
    }
}

#[allow(clippy::similar_names)]
fn relative_import_dir(importer: Pid, importee: Pid) -> Option<Utf8PathBuf> {
    let importer_dir = &importer.package().manifest_path.parent()?;
    let importee_dir = &importee.package().manifest_path.parent()?;
    pathdiff::diff_utf8_paths(importee_dir, importer_dir)
}

#[derive(Debug)]
pub struct ChangePackage<'a> {
    pub name: String,
    pub ty: Ty,
    pub version: Version,
    pub source: PackageSource<'a>,
    pub feats: BTreeSet<String>,
    pub rename: bool,
}

impl PackageSource<'_> {
    pub fn insert_into(&self, ver: &Version, table: &mut toml_edit::InlineTable) {
        match self {
            PackageSource::Registry(_) => {
                table.insert("version", toml_edit::Value::from(ver.to_string()));
            }
            PackageSource::Git(_) => todo!(),
            PackageSource::File { path } => {
                table.insert("path", toml_edit::Value::from(path.to_string()));
            }
        }
    }
}

#[derive(Debug, Hash)]
#[allow(clippy::module_name_repetitions)]
pub enum PackageSource<'a> {
    Registry(&'a str),
    Git(&'a str),
    File { path: Utf8PathBuf },
}

impl PackageSource<'_> {
    pub const CRATES_IO: Self =
        PackageSource::Registry("https://github.com/rust-lang/crates.io-index");
}

impl std::fmt::Display for PackageSource<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackageSource::Registry(_reg) => f.write_str("registry"),
            PackageSource::Git(url) => write!(f, "{url}"),
            PackageSource::File { path } => path.fmt(f),
        }
    }
}
