use anyhow::Context;
use guppy::graph::ExternalSource;
use guppy::Version;
use guppy::{graph::PackageGraph, PackageId};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use toml_edit::{value, Array, Document, InlineTable, Item, Table, Value};
use tracing::debug;

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
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();

    Hash::hash(&table.to_string(), &mut hasher);
    Hasher::finish(&hasher) as i64
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

    let stash_table = match get_table(toml, &["stash"])?.remove("dependencies") {
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

    let lock_table = get_table(&mut toml, &["package", "metadata", "hackerman", "lock"])?;
    let lock = lock_table
        .get(kind)
        .ok_or_else(|| {
            anyhow::anyhow!("Couldn't get saved lock value for {kind} in {manifest_path:?}",)
        })?
        .as_integer()
        .expect("Invalid checksum format for {kind} in {manifest_path:?}");

    if lock != checksum {
        debug!("Expected: {lock}, actual {checksum}");
        anyhow::bail!("Checksum mismatch for {kind} in {manifest_path:?}")
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn set_dependencies_ext_crates() -> anyhow::Result<()> {
        let mut toml = "[dependencies]".parse::<Document>()?;
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
parsergen = false
";
        assert_eq!(toml.to_string(), expected);

        Ok(())
    }
}
