#![allow(clippy::missing_errors_doc)]

use anyhow::Context;
use cargo_metadata::camino::{Utf8Path, Utf8PathBuf};
use std::hash::{Hash, Hasher};
use std::ops::{Index, IndexMut};
use std::path::Path;
use toml_edit::{value, Array, Decor, Document, InlineTable, Item, Table, Value};
use tracing::{debug, info};

use crate::hack::Ty;
use crate::source::ChangePackage;

const BANNER: &str = r"# !
# ! This Cargo.toml file has unified features. In order to edit it
# ! you should first restore it using `cargo hackerman restore` command
# !

";

pub fn set_dependencies(
    path: &Utf8PathBuf,
    lock: bool,
    changes: &[ChangePackage],
) -> anyhow::Result<()> {
    info!("updating {path}");
    let mut toml = std::fs::read_to_string(path)?.parse::<Document>()?;

    set_dependencies_toml(&mut toml, lock, changes)?;
    std::fs::write(&path, toml.to_string())?;
    Ok(())
}

fn get_decor(toml: &mut Document) -> anyhow::Result<&mut Decor> {
    let (_key, item) = toml
        .as_table_mut()
        .iter_mut()
        .next()
        .ok_or_else(|| anyhow::anyhow!("Empty toml document?"))?;

    Ok(match item {
        Item::None => anyhow::bail!("Empty toml document?"),
        Item::Value(val) => val.decor_mut(),
        Item::Table(val) => val.decor_mut(),
        Item::ArrayOfTables(val) => val
            .get_mut(0)
            .ok_or_else(|| anyhow::anyhow!("Empty toml document?"))?
            .decor_mut(),
    })
}

fn add_banner(toml: &mut Document) -> anyhow::Result<()> {
    let decor = get_decor(toml)?;
    match decor.prefix() {
        Some(old) => {
            let new = format!("{BANNER}{old}");
            decor.set_prefix(new);
        }
        None => decor.set_prefix(BANNER),
    }
    Ok(())
}

fn strip_banner(toml: &mut Document) -> anyhow::Result<bool> {
    let decor = get_decor(toml)?;
    Ok(match decor.prefix() {
        Some(cur) => {
            if let Some(rest) = cur.strip_prefix(BANNER) {
                let new = rest.to_string();
                decor.set_prefix(new);
                false
            } else {
                true
            }
        }
        None => false,
    })
}

const HACKERMAN_PATH: &[&str] = &["package", "metadata", "hackerman"];
const LOCK_PATH: &[&str] = &["package", "metadata", "hackerman", "lock"];
const STASH_PATH: &[&str] = &["package", "metadata", "hackerman", "stash"];
const NORM_STASH_PATH: &[&str] = &["package", "metadata", "hackerman", "stash", "dependencies"];
#[rustfmt::skip]
const DEV_STASH_PATH: &[&str] = &["package", "metadata", "hackerman", "stash", "dev-dependencies"];

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

fn add_checksum<H: Hasher>(item: &Item, hasher: &mut H) -> anyhow::Result<()> {
    match item {
        Item::None => {}
        Item::Value(value) => Hash::hash(&value.to_string(), hasher),
        Item::Table(t) => {
            for (k, v) in t.iter() {
                Hash::hash(k, hasher);
                add_checksum(v, hasher)?;
            }
        }
        Item::ArrayOfTables(t) => {
            for table in t.iter() {
                for (k, v) in table.iter() {
                    Hash::hash(k, hasher);
                    add_checksum(v, hasher)?;
                }
            }
        }
    }
    Ok(())
}

fn get_checksum(toml: &Document) -> anyhow::Result<i64> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();

    let t = match toml.as_item() {
        Item::Table(t) => t,
        Item::None | Item::Value(_) | Item::ArrayOfTables(_) => anyhow::bail!("bogus toml"),
    };

    for (name, item) in t.iter() {
        match name {
            "dependencies" | "dev-dependencies" | "build-dependencies" | "target" => {
                add_checksum(item, &mut hasher)?
            }
            _ => debug!("Skipping toml key {name:?} while calculating checksum"),
        }
    }

    // keep numbers positive
    Ok(i64::try_from(
        Hasher::finish(&hasher) % 8_000_000_000_000_000_000,
    )?)
}

fn compile_change_package(change: &ChangePackage) -> (Item, String) {
    let mut new = InlineTable::new();
    change.source.insert_into(&change.version, &mut new);
    let feats = change
        .feats
        .iter()
        .filter(|&f| f != "default")
        .collect::<Array>();
    if !feats.is_empty() {
        new.insert("features", Value::from(feats));
    }
    if !change.feats.contains("default") {
        new.insert("default-features", Value::from(false));
    }

    let new_name = if change.rename {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        Hash::hash(&change.source, &mut hasher);
        let hash = Hasher::finish(&hasher);
        new.insert("package", Value::from(&change.name));
        format!("hackerman-{}-{}", &change.name, hash)
    } else {
        change.name.clone()
    };
    (value(new), new_name)
}

#[derive(Default)]
struct Stash {
    norm: Vec<(String, Item)>,
    dev: Vec<(String, Item)>,
}

impl Index<Ty> for Stash {
    type Output = Vec<(String, Item)>;

    fn index(&self, index: Ty) -> &Self::Output {
        match index {
            Ty::Dev => &self.dev,
            Ty::Norm => &self.norm,
        }
    }
}

impl IndexMut<Ty> for Stash {
    fn index_mut(&mut self, index: Ty) -> &mut Self::Output {
        match index {
            Ty::Dev => &mut self.dev,
            Ty::Norm => &mut self.norm,
        }
    }
}

fn set_dependencies_toml(
    toml: &mut Document,
    lock: bool,
    changes: &[ChangePackage],
) -> anyhow::Result<bool> {
    let mut was_modified = false;
    if toml.contains_key("target") {
        anyhow::bail!("target filtered dependencies present in the workspace are not supported by split mode hack")
    }
    let mut saved = Stash::default();

    for change in changes {
        let top = change.ty.table_name();
        let table = get_table(toml, &[top])?;
        let (item, name) = compile_change_package(change);
        let old = table.insert(&name, item).unwrap_or_else(|| value(false));
        saved[change.ty].push((name, old))
    }

    if lock {
        was_modified = true;
        let hash = get_checksum(toml)?;
        let lock_table = get_table(toml, LOCK_PATH)?;
        lock_table.insert("dependencies", value(hash));
        lock_table.sort_values();
        lock_table.set_position(997);
    }

    let stash = get_table(toml, NORM_STASH_PATH)?;
    stash.set_position(998);
    for (name, val) in saved.norm {
        stash.insert(&name, val);
    }

    let dev_stash = get_table(toml, DEV_STASH_PATH)?;
    dev_stash.set_position(999);
    for (name, val) in saved.dev {
        dev_stash.insert(&name, val);
    }
    if was_modified {
        add_banner(toml)?;
    }
    Ok(was_modified)
}

pub fn restore_path(manifest_path: &Path) -> anyhow::Result<bool> {
    let mut toml = std::fs::read_to_string(&manifest_path)?.parse::<Document>()?;
    let changed = restore_toml(&mut toml)?;
    if changed {
        std::fs::write(&manifest_path, toml.to_string())?;
    }
    Ok(changed)
}

pub fn restore(manifest_path: &Utf8Path) -> anyhow::Result<bool> {
    let mut toml = std::fs::read_to_string(&manifest_path)?.parse::<Document>()?;

    info!("Restoring {manifest_path}");
    let changed = restore_toml(&mut toml).with_context(|| format!("in {manifest_path}"))?;
    if changed {
        std::fs::write(&manifest_path, toml.to_string())?;
    } else {
        debug!("No changes to {manifest_path}");
    }

    Ok(changed)
}

fn restore_toml(toml: &mut Document) -> anyhow::Result<bool> {
    let hackerman = get_table(toml, HACKERMAN_PATH)?;
    let mut changed = hackerman.remove("lock").is_some();

    for ty in ["dependencies", "dev-dependencies"] {
        let stash = match get_table(toml, STASH_PATH)?.remove(ty) {
            Some(Item::Table(t)) => t,
            Some(_) => anyhow::bail!("corrupted stash table"),
            None => continue,
        };

        let table = get_table(toml, &[ty])?;
        for (key, item) in stash {
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
    }
    changed |= strip_banner(toml)?;
    Ok(changed)
}

pub fn verify_checksum(manifest_path: &Path) -> anyhow::Result<()> {
    let mut toml = std::fs::read_to_string(&manifest_path)?.parse::<Document>()?;

    let checksum = get_checksum(&toml)?;

    let lock_table = get_table(&mut toml, LOCK_PATH)?;
    if lock_table.is_empty() {
        return Ok(());
    }
    if lock_table
        .get("dependencies")
        .and_then(Item::as_integer)
        .map_or(false, |l| l == checksum)
    {
        anyhow::bail!("Checksum mismatch in {manifest_path:?}")
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use cargo_metadata::Version;

    use crate::source::PackageSource;

    use super::*;

    #[test]
    fn target_specific_feats() -> anyhow::Result<()> {
        let toml = r#"
[target.'cfg(target_os = "android")'.dependencies]
package = 1.0
"#
        .parse::<Document>()?;

        let hash = get_checksum(&toml)?;
        assert_eq!(hash, 2329902156198620770);
        Ok(())
    }

    #[test]
    fn odd_declarations_are_supported() -> anyhow::Result<()> {
        let toml = r#"
[dependencies]
by_version_1 = "1.0"
by_version_2 = { version = "1.0", features = ["one", "two"] }
from_git = { git = "https://github.com/rust-lang/regex" }
"#
        .parse::<Document>()?;

        let hash = get_checksum(&toml)?;

        assert_eq!(hash, 559992462246589769);
        Ok(())
    }

    #[test]
    fn fancy_declarations_are_working() -> anyhow::Result<()> {
        let toml1 = "[dependencies.fancy]\nversion = \"1.0\"".parse()?;
        let toml2 = "[dependencies.fancy]\nversion = \"1.2\"".parse()?;
        assert_ne!(get_checksum(&toml1)?, get_checksum(&toml2)?);

        Ok(())
    }

    #[test]
    fn lock_removal_works() -> anyhow::Result<()> {
        let mut toml = "[package.metadata.hackerman.lock]\ndependencies = 1".parse()?;
        restore_toml(&mut toml)?;
        assert_eq!(toml.to_string(), "");
        Ok(())
    }

    #[test]
    fn lock_removal_works_without_lock_present() -> anyhow::Result<()> {
        let mut toml = "".parse()?;
        restore_toml(&mut toml)?;
        assert_eq!(toml.to_string(), "");
        Ok(())
    }

    #[test]
    fn add_banner_works() -> anyhow::Result<()> {
        let s = r#"
[dependencies]
version = 1.0

[dev-dependencies]
"#;
        let mut toml = s.parse()?;
        add_banner(&mut toml)?;
        let expected = format!("{BANNER}{s}");
        assert_eq!(expected, toml.to_string());
        Ok(())
    }

    #[test]
    fn set_dependencies_works_0() -> anyhow::Result<()> {
        let mut toml = r#"
[dependencies]
package = 1.0
"#
        .parse::<Document>()?;

        let mut feats = BTreeSet::new();
        feats.insert("dummy".to_string());

        let changes = [ChangePackage {
            name: "package".to_string(),
            ty: Ty::Norm,
            version: Version::new(1, 0, 0),
            source: PackageSource::CRATES_IO,
            feats,
            rename: false,
        }];

        set_dependencies_toml(&mut toml, false, &changes)?;

        todo!("{toml}");

        Ok(())
    }

    #[test]
    fn set_dependencies_works_1() -> anyhow::Result<()> {
        let mut toml = r#"
[target.'cfg(target_os = "linux")'.dependencies]
package = 1.0
"#
        .parse::<Document>()?;

        let mut feats = BTreeSet::new();
        feats.insert("dummy".to_string());

        let changes = [ChangePackage {
            name: "package".to_string(),
            ty: Ty::Norm,
            version: Version::new(1, 0, 0),
            source: PackageSource::CRATES_IO,
            feats,
            rename: false,
        }];

        set_dependencies_toml(&mut toml, false, &changes)?;

        todo!("{toml}");

        Ok(())
    }
}
