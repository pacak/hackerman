use guppy::{graph::PackageGraph, PackageId};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use toml_edit::{value, Array, Document, InlineTable, Item, Table, Value};
use tracing::debug;

const HACKERMAN_PATH: &[&str] = &["package", "metadata", "hackerman"];

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

pub fn set_dependencies<P>(
    manifest_path: P,
    g: &PackageGraph,
    patch: &BTreeMap<&PackageId, BTreeSet<&str>>,
    lock: bool,
) -> anyhow::Result<()>
where
    P: AsRef<Path> + std::fmt::Debug,
{
    let kind = "dependencies";
    let mut toml = std::fs::read_to_string(&manifest_path)?.parse::<Document>()?;

    let stash = (|| {
        toml.get("package")?
            .as_table()?
            .get("hackerman")?
            .as_table()?
            .get("stash")
    })();
    if stash.is_some() {
        anyhow::bail!(
            "{:?} already contains changes, restore the original files before applying a new hack",
            manifest_path
        );
    }

    let table = get_table(&mut toml, &[kind])?;
    let mut changes = Vec::new();
    for (package_id, feats) in patch.iter() {
        let dep = g.metadata(package_id)?;
        let name = dep.name();

        let mut new_dep = InlineTable::new();
        new_dep.insert("version", dep.version().to_string().into());
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
        let lock_table = get_table(&mut toml, &["package", "metadata", "hackerman", "lock"])?;
        lock_table.insert(kind, value(hash));
        lock_table.sort_values();
        lock_table.set_position(998);
    }

    let stash_table = get_table(
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

    Ok(())
}

pub fn restore_dependencies<P>(manifest_path: P) -> anyhow::Result<()>
where
    P: AsRef<Path> + std::fmt::Debug,
{
    let mut toml = std::fs::read_to_string(&manifest_path)?.parse::<Document>()?;

    let kind = "dependencies";

    let mut has_lock = false;
    if let Some(t) = toml["package"]["metadata"]["hackerman"].as_table_mut() {
        t.remove("lock");
        has_lock = true;
    }

    let stash_table = match toml["package"]["metadata"]["hackerman"]["stash"].as_table_mut() {
        Some(table) => match table.remove(kind) {
            Some(Item::Table(table)) => Some(table),
            Some(_) => anyhow::bail!("corrupted stash table in {:?}", manifest_path),
            None => None,
        },
        None => None,
    };

    let stash_table = match stash_table {
        Some(t) => t,
        None => {
            if has_lock {
                std::fs::write(&manifest_path, toml.to_string())?;
            }
            return Ok(());
        }
    };

    let table = get_table(&mut toml, &[kind])?;
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
    }
    table.sort_values();
    std::fs::write(&manifest_path, toml.to_string())?;
    Ok(())
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
