use anyhow::Context;
use guppy::graph::ExternalSource;
use guppy::Version;
use guppy::{graph::PackageGraph, PackageId};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use toml_edit::{value, Array, Document, InlineTable, Item, Table, Value};
use tracing::debug;

use crate::feat_graph::Pid;

const HACKERMAN_PATH: &[&str] = &["package", "metadata", "hackerman"];
const LOCK_PATH: &[&str] = &["package", "metadata", "hackerman", "lock"];
const STASH_PATH: &[&str] = &["package", "metadata", "hackerman", "stash", "dependencies"];

fn get_table<'a>(mut table: &'a mut Table, path: &[&str]) -> anyhow::Result<&'a mut Table> {
    for (ix, comp) in path.iter().enumerate() {
        table = table
            .entry(comp)
            .or_insert_with(toml_edit::table)
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("Expected table at path {}", path[..ix].join(".")))?;
        table.set_implicit(true);
    }
    Ok(table)
}

fn get_checksum(table: &Table) -> i64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&table.to_string(), &mut hasher);
    std::hash::Hasher::finish(&hasher) as i64
}

fn set_dependencies_toml<'a, 'b, I>(
    toml: &'b mut Document,
    lock: bool,
    changes: I,
) -> anyhow::Result<bool>
where
    I: Iterator<
        Item = anyhow::Result<(
            &'a str,
            &'a Version,
            ExternalSource<'a>,
            &'a BTreeSet<&'a str>,
        )>,
    >,
{
    let mut res = Vec::new();
    let mut changed = false;

    let table = get_table(toml, &["dependencies"])?;

    for x in changes {
        let (name, version, src, feats) = x?;
        changed |= true;

        let mut new_dep = InlineTable::new();
        match src {
            ExternalSource::Registry("https://github.com/rust-lang/crates.io-index") => {
                new_dep.insert("version", version.to_string().into());
            }
            ExternalSource::Git {
                repository: _,
                req: _,
                resolved: _,
            } => todo!(),
            unsupported => anyhow::bail!("Unsupported source: {:?}", unsupported),
        }

        let mut feats_arr = Array::new();
        feats_arr.extend(feats.iter().copied().filter(|&f| f != "default"));
        if !feats_arr.is_empty() {
            new_dep.insert("features", Value::Array(feats_arr));
        }
        if !feats.contains("default") {
            new_dep.insert("default-features", false.into());
        }

        res.push((name, table.insert(name, value(new_dep))));
    }
    table.sort_values();

    if lock {
        changed |= true;
        let hash = get_checksum(table);
        let lock_table = get_table(toml, LOCK_PATH)?;
        lock_table.insert("dependencies", value(hash));
        lock_table.sort_values();
        lock_table.set_position(998);
    }

    let stash_table = get_table(toml, STASH_PATH)?;
    if !stash_table.is_empty() {
        anyhow::bail!(
            "Manifest contains changes, restore the original files before applying a new hack",
        );
    }

    for (name, old) in res.into_iter() {
        match old {
            Some(t) => stash_table.insert(name, t),
            None => stash_table.insert(name, value(false)),
        };
    }
    stash_table.sort_values();
    stash_table.set_position(999);
    Ok(changed)
}

struct Cfg {
    lock: bool,
    banner: bool,
}

pub fn set_dependencies2(
    package: &cargo_metadata::Package,
    patch: BTreeMap<Pid, BTreeSet<&str>>,
) -> anyhow::Result<()> {
    let mut toml = std::fs::read_to_string(&package.manifest_path)?.parse::<Document>()?;
    let patches = patch
        .iter()
        .map(|(pid, feats)| {
            let dep_package = pid.package();
            let name = &dep_package.name;
            let src = &dep_package.source.as_ref().unwrap().repr;
            let source = guppy::graph::ExternalSource::new(src).unwrap();
            (name, source)
        })
        .collect::<Vec<_>>();

    todo!("{:?}", patches);

    Ok(())
}

pub fn set_dependencies<P>(
    manifest_path: P,
    g: &PackageGraph,
    patch: &BTreeMap<&PackageId, BTreeSet<&str>>,
    lock: bool,
) -> anyhow::Result<()>
where
    P: AsRef<Path> + std::fmt::Debug,
{
    let mut toml = std::fs::read_to_string(&manifest_path)?.parse::<Document>()?;

    let patches = patch.iter().map(|(package_id, feats)| {
        let package = g.metadata(package_id)?;
        let name = package.name();
        let version = package.version();
        let src = package
            .source()
            .parse_external()
            .ok_or_else(|| anyhow::anyhow!("not an external thing"))?;
        Ok((name, version, src, feats))
    });

    let changed = set_dependencies_toml(&mut toml, lock, patches)
        .with_context(|| format!("Manifest {:?}", manifest_path))?;

    if changed {
        std::fs::write(&manifest_path, toml.to_string())?;
    }

    /*
    <<<<<<< HEAD
    =======
        let table = to_table(&mut toml, &[kind])?;
        let mut changes = Vec::new();
        for (package_id, feats) in patch.iter() {
            let dep = g.metadata(package_id)?;
            let name = dep.name();

            todo!("look at {:?}", dep.source().parse_external());
            let mut new_dep = InlineTable::new();
            let semver = dep.version();
            let version = format!("{}.{}.{}", semver.major, semver.minor, semver.patch);
            new_dep.insert("version", version.into());
            let mut feats_arr = Array::new();
            feats_arr.extend(feats.iter().copied().filter(|&f| f != "default"));
            if !feats_arr.is_empty() {
                new_dep.insert("features", Value::Array(feats_arr));
            }
            if !feats.contains("default") {
                new_dep.insert("default-features", false.into());
            }

            changes.push((name, table.insert(name, value(new_dep))));
        }
        table.sort_values();

        if lock {
            let hash = get_checksum(table);
            let lock_table = to_table(&mut toml, &["package", "metadata", "hackerman", "lock"])?;
            lock_table.insert(kind, value(hash));
            lock_table.sort_values();
            lock_table.set_position(998);
        }

        let stash_table = to_table(
            &mut toml,
            &["package", "metadata", "hackerman", "stash", kind],
        )?;
        for (name, old) in changes {
            match old {
                Some(t) => stash_table.insert(name, t),
                None => stash_table.insert(name, value(false)),
            };
        }
        stash_table.sort_values();
        stash_table.set_position(999);

        std::fs::write(&manifest_path, toml.to_string())?;

    >>>>>>> 62f396f */
    Ok(())
}

pub fn restore_dependencies<P>(manifest_path: P) -> anyhow::Result<()>
where
    P: AsRef<Path> + std::fmt::Debug,
{
    let mut toml = std::fs::read_to_string(&manifest_path)?.parse::<Document>()?;

    let changed = restore_dependencies_toml(&mut toml)?;
    if changed {
        std::fs::write(&manifest_path, toml.to_string())?;
    }
    Ok(())
}

fn restore_dependencies_toml(toml: &mut Document) -> anyhow::Result<bool> {
    let hackerman = get_table(toml, HACKERMAN_PATH)?;
    let mut changed = hackerman.remove("lock").is_some();

    let stash_table = match get_table(hackerman, &["stash"])?.remove("dependencies") {
        Some(Item::Table(t)) => t,
        Some(_) => anyhow::bail!("corrupted stash table"),
        None => return Ok(changed),
    };

    let table = get_table(toml, &["dependencies"])?;
    for (key, item) in stash_table.into_iter() {
        if item.is_inline_table() || item.is_str() {
            debug!("Restoring dependency {}: {}", key, item.to_string());
            table.insert(&key, item);
        } else if item.is_bool() {
            debug!("Removing dependency {}", key);
            table.remove(&key);
        } else {
            anyhow::bail!("Corrupted key {:?}: {}", key, item.to_string());
        }
        changed = true;
    }
    table.sort_values();

    Ok(changed)
}

pub fn verify_checksum<P>(manifest_path: P) -> anyhow::Result<()>
where
    P: AsRef<Path> + std::fmt::Debug,
{
    let kind = "dependencies";
    let mut toml = std::fs::read_to_string(&manifest_path)?.parse::<Document>()?;
    let table = get_table(&mut toml, &[kind])?;

    let checksum = get_checksum(table);

    let lock_table = get_table(&mut toml, LOCK_PATH)?;
    if lock_table.is_empty() {
        return Ok(());
    }
    if lock_table
        .get(kind)
        .and_then(|x| x.as_integer())
        .map_or(false, |l| l == checksum)
    {
        anyhow::bail!("Checksum mismatch in {manifest_path:?}")
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odd_declarations_are_supported() -> anyhow::Result<()> {
        let mut s = "\

[dependencies]
by_version_1 = \"1.0\"
by_version_2 = { version = \"1.0\" }
from_git = { git = \"https://github.com/rust-lang/regex\" }

[dependencies.fancy]
version = \"1.0\"
"
        .parse::<Document>()?;

        todo!("{:?}", s);
    }

    #[test]
    fn lock_removal_works() -> anyhow::Result<()> {
        let mut s = "[package.metadata.hackerman.lock]\ndependencies = 1".parse()?;
        restore_dependencies_toml(&mut s)?;
        assert_eq!(s.to_string(), "");
        Ok(())
    }

    #[test]
    fn lock_removal_works2() -> anyhow::Result<()> {
        let mut s = "".parse()?;
        restore_dependencies_toml(&mut s)?;
        assert_eq!(s.to_string(), "");
        Ok(())
    }

    #[test]
    fn set_and_restore_dependencies_ext_crates() -> anyhow::Result<()> {
        let original = "\
[dependencies]
parsergen = { version = \"1.1.1\",  default-features = false }
";
        let mut toml = original.parse::<Document>()?;
        let version = "1.1.1".parse::<Version>()?;
        let feats = ["derive"].iter().copied().collect::<BTreeSet<_>>();
        let src = ExternalSource::Registry("https://github.com/rust-lang/crates.io-index");
        let deps = [Ok(("parsergen", &version, src, &feats))];
        set_dependencies_toml(&mut toml, true, deps.into_iter())?;
        let expected = "\
[dependencies]
parsergen = { version = \"1.1.1\", features = [\"derive\"], default-features = false }

[package.metadata.hackerman.lock]
dependencies = -6893235233160425550

[package.metadata.hackerman.stash.dependencies]
parsergen = { version = \"1.1.1\",  default-features = false }
";
        assert_eq!(toml.to_string(), expected);

        let changed = restore_dependencies_toml(&mut toml)?;
        assert!(changed);
        assert_eq!(toml.to_string(), original);

        Ok(())
    }
}
