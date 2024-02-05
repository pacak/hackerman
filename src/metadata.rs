use cargo_metadata::Dependency;
use cargo_platform::Cfg;

use crate::{feat_graph::Feature, hack::Collect};

#[derive(Eq, PartialEq, Clone, Debug, Copy, Hash, PartialOrd, Ord)]
/// Dependencies can come in three kinds
pub enum DependencyKind {
    /// The 'normal' kind
    Normal,
    /// Those used in tests only
    Development,
    /// Those used in build scripts only
    Build,
    Unknown,
}

impl From<cargo_metadata::DependencyKind> for DependencyKind {
    fn from(x: cargo_metadata::DependencyKind) -> Self {
        match x {
            cargo_metadata::DependencyKind::Normal => DependencyKind::Normal,
            cargo_metadata::DependencyKind::Development => DependencyKind::Development,
            cargo_metadata::DependencyKind::Build => DependencyKind::Build,
            _ => DependencyKind::Unknown,
        }
    }
}

#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct DepKindInfo {
    pub kind: DependencyKind,
    pub target: Option<cargo_platform::Platform>,
}

impl DepKindInfo {
    pub const NORMAL: Self = Self {
        kind: DependencyKind::Normal,
        target: None,
    };

    pub const DEV: Self = Self {
        kind: DependencyKind::Development,
        target: None,
    };

    fn satisfies(
        &self,
        source: Feature,
        filter: Collect,
        platforms: &[&str],
        cfgs: &[Cfg],
    ) -> bool {
        if self.kind == DependencyKind::Development {
            match filter {
                Collect::AllTargets | Collect::Target | Collect::NoDev | Collect::NormalOnly => {
                    return false
                }
                Collect::MemberDev(pid) => {
                    if let Some(this_fid) = source.fid() {
                        {
                            if this_fid.pid != pid {
                                return false;
                            }
                        }
                    }
                }
                Collect::DevTarget => {
                    if !source.is_workspace() {
                        return false;
                    }
                }
            };
        }

        self.target
            .as_ref()
            .map_or(true, |p| p.matches(platforms[0], cfgs))
    }
}

impl From<&Dependency> for DepKindInfo {
    fn from(dep: &Dependency) -> Self {
        Self {
            kind: dep.kind.into(),
            target: dep.target.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Link {
    /// if dependency is specified as optional or required
    pub optional: bool,
    pub kinds: Vec<DepKindInfo>,
}

impl Link {
    /// unconditional link
    pub const ALWAYS: Link = Link {
        optional: false,
        kinds: Vec::new(),
    };

    /// optional lib dependency
    pub const OPT: Link = Link {
        optional: true,
        kinds: Vec::new(),
    };

    pub(crate) fn is_dev_only(&self) -> bool {
        self.kinds
            .iter()
            .all(|k| k.kind == DependencyKind::Development)
    }
    pub(crate) fn is_normal(&self) -> bool {
        self.kinds.iter().any(|k| k.kind == DependencyKind::Normal)
    }

    pub(crate) fn satisfies(
        &self,
        source: Feature,
        filter: Collect,
        platforms: &[&str],
        cfgs: &[Cfg],
    ) -> bool {
        self.kinds
            .iter()
            .any(|kind| kind.satisfies(source, filter, platforms, cfgs))
    }
}
