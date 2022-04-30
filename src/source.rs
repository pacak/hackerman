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

    use super::optimize_feats;
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
}

impl ChangePackage {
    pub fn make(
        importer: Pid,
        importee: Pid,
        ty: Ty,
        rename: bool,
        mut feats: BTreeSet<String>,
    ) -> Self {
        let package = importee.package();
        optimize_feats(&package.features, &mut feats);
        if let Some(src) = &package.source {
            if src.is_crates_io() {
                ChangePackage {
                    name: package.name.clone(),
                    ty,
                    source: PackageSource::Registry(package.version.clone()),
                    feats,
                    rename,
                }
            } else if src.to_string().starts_with("path+file:") {
                todo!("path import");
            } else {
                todo!(
                    "{:?}\n{:?}\n{:?}\nSource {src:?} is not supported yet",
                    importer,
                    importee,
                    feats
                );
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
                source,
                feats,
                rename,
            }
        }
    }
}

fn relative_import_dir(importer: Pid, importee: Pid) -> Option<Utf8PathBuf> {
    let importer_dir = &importer.package().manifest_path.parent()?;
    let importee_dir = &importee.package().manifest_path.parent()?;
    pathdiff::diff_utf8_paths(importee_dir, importer_dir)
}

#[derive(Debug)]
pub struct ChangePackage {
    pub name: String,
    pub ty: Ty,
    pub source: PackageSource,
    pub feats: BTreeSet<String>,
    pub rename: bool,
}

impl PackageSource {
    pub fn insert_into(&self, table: &mut toml_edit::InlineTable) {
        match self {
            PackageSource::Registry(ver) => {
                table.insert("version", toml_edit::Value::from(ver.to_string()));
            }
            PackageSource::Git { url: _, ver: _ } => todo!(),
            PackageSource::File { path } => {
                table.insert("path", toml_edit::Value::from(path.to_string()));
            }
        }
    }
}

#[derive(Debug, Hash)]
#[allow(clippy::module_name_repetitions)]
pub enum PackageSource {
    Registry(Version),
    Git { url: String, ver: GitVersion },
    File { path: Utf8PathBuf },
}

impl std::fmt::Display for GitVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitVersion::Branch(b) => write!(f, "branch: {b}"),
            GitVersion::Tag(b) => write!(f, "tag: {b}"),
            GitVersion::Rev(b) => write!(f, "rev: {b}"),
        }
    }
}
impl std::fmt::Display for PackageSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackageSource::Registry(ver) => ver.fmt(f),
            PackageSource::Git { url, ver } => write!(f, "{url} {ver}"),
            PackageSource::File { path } => path.fmt(f),
        }
    }
}

#[derive(Debug, Hash)]
pub enum GitVersion {
    Branch(String),
    Tag(String),
    Rev(String),
}
