use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use guppy::{graph::PackageGraph, DependencyKind};

use crate::NonMacroKind;

pub fn list(package_graph: &PackageGraph, kind: DependencyKind) -> anyhow::Result<()> {
    let mut items = BTreeMap::new();

    for p in package_graph
        .query_workspace()
        .resolve_with(NonMacroKind(kind))
        .packages(guppy::graph::DependencyDirection::Forward)
    {
        items
            .entry(p.name())
            .or_insert_with(BTreeSet::new)
            .insert(p.version());
    }

    let mut buf = String::new();
    for (package, versions) in items {
        if versions.len() == 1 {
            continue;
        }
        write!(buf, "{}:", package)?;
        for v in versions {
            write!(buf, " v{},", v)?;
        }
        buf.pop();
        writeln!(buf)?;
    }
    print!("{}", buf);

    Ok(())
}
