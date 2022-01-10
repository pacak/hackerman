use guppy::{graph::PackageGraph, PackageId};
use std::{collections::BTreeMap, path::Path};
use toml_edit::{table, value, Array, Document, InlineTable, Table, Value};

fn to_table<'a>(toml: &'a mut Document, path: &[&str]) -> anyhow::Result<&'a mut Table> {
    let mut entry = toml
        .entry(path[0])
        .or_insert_with(table)
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("Expected table"))?;
    entry.set_implicit(true);
    for comp in &path[1..] {
        entry = entry
            .entry(comp)
            .or_insert_with(table)
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("Expected table"))?;
        entry.set_implicit(true);
    }

    Ok(entry)
}
pub fn set_dependencies<P>(
    manifest_path: P,
    g: &PackageGraph,
    patch: &BTreeMap<&PackageId, Vec<&str>>,
) -> anyhow::Result<()>
where
    P: AsRef<Path> + std::fmt::Debug,
{
    let kind = "dependencies";
    let mut toml = std::fs::read_to_string(&manifest_path)?.parse::<Document>()?;

    for (package_id, feats) in patch.iter() {
        let dep = g.metadata(package_id)?;
        let name = dep.name();

        let table = to_table(&mut toml, &[kind])?;

        let mut new_dep = InlineTable::new();
        new_dep.insert("version", dep.version().to_string().into());
        let mut feats_arr = Array::new();
        feats_arr.extend(feats.iter().copied());
        new_dep.insert("features", Value::Array(feats_arr));

        let old = table.insert(name, value(new_dep));
        let bak = to_table(&mut toml, &["package", "metadata", "hackerman", kind])?;
        match old {
            Some(t) => bak.insert(name, t),
            None => bak.insert(name, value(false)),
        };

        bak.set_position(999);
    }

    to_table(&mut toml, &[kind])?.sort_values();
    std::fs::write(&manifest_path, toml.to_string())?;

    Ok(())
}
