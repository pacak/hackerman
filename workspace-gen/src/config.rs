use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Deserialize)]
pub struct WorkspaceConfig {
    pub workspace: Workspace,
    #[serde(default)]
    pub crates: Vec<CrateConfig>,
}

impl WorkspaceConfig {
    pub fn merge_duplicates(mut self) -> Self {
        let mut merged: BTreeMap<(String, String), CrateConfig> = BTreeMap::new();

        for crate_config in self.crates.drain(..) {
            let key = (crate_config.name.clone(), crate_config.version.clone());
            match merged.get_mut(&key) {
                Some(existing) => {
                    // Merge dependencies
                    for (k, v) in crate_config.dependencies {
                        existing.dependencies.entry(k).or_insert(v);
                    }
                    for (k, v) in crate_config.build_dependencies {
                        existing.build_dependencies.entry(k).or_insert(v);
                    }
                    for (k, v) in crate_config.dev_dependencies {
                        existing.dev_dependencies.entry(k).or_insert(v);
                    }
                    // Merge features (last wins for duplicates)
                    for (k, v) in crate_config.features {
                        existing.features.insert(k, v);
                    }
                    // Merge platform-specific deps
                    for platform in crate_config.platform {
                        existing.platform.push(platform);
                    }
                }
                None => {
                    merged.insert(key, crate_config);
                }
            }
        }

        self.crates = merged.into_values().collect();
        self
    }
}

#[derive(Debug, Deserialize)]
pub struct Workspace {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct CrateConfig {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub workspace_member: bool,
    #[serde(default)]
    pub proc_macro: bool,
    #[serde(default)]
    pub dependencies: BTreeMap<String, DependencySpec>,
    #[serde(default)]
    pub build_dependencies: BTreeMap<String, DependencySpec>,
    #[serde(default)]
    pub dev_dependencies: BTreeMap<String, DependencySpec>,
    #[serde(default)]
    pub features: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub platform: Vec<PlatformConfig>,
}

fn default_version() -> String {
    "0.1.0".to_string()
}

#[derive(Debug, Clone)]
pub enum DependencySpec {
    Config(DependencyConfig),
    Features(Vec<String>),
}

impl DependencySpec {
    pub fn to_config(&self) -> DependencyConfig {
        match self {
            DependencySpec::Config(cfg) => cfg.clone(),
            DependencySpec::Features(features) => DependencyConfig {
                version: None,
                optional: None,
                features: Some(features.clone()),
                default_features: None,
                path: None,
            },
        }
    }
}

impl<'de> Deserialize<'de> for DependencySpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct DependencySpecVisitor;

        impl<'de> Visitor<'de> for DependencySpecVisitor {
            type Value = DependencySpec;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a map or a sequence")
            }

            fn visit_seq<A>(self, seq: A) -> Result<DependencySpec, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let vec = Deserialize::deserialize(de::value::SeqAccessDeserializer::new(seq))?;
                Ok(DependencySpec::Features(vec))
            }

            fn visit_map<A>(self, map: A) -> Result<DependencySpec, A::Error>
            where
                A: MapAccess<'de>,
            {
                let cfg = Deserialize::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(DependencySpec::Config(cfg))
            }
        }

        deserializer.deserialize_any(DependencySpecVisitor)
    }
}

#[derive(Debug, Deserialize)]
pub struct PlatformConfig {
    pub target: String,
    #[serde(default)]
    pub dependencies: BTreeMap<String, DependencySpec>,
    #[serde(default)]
    pub build_dependencies: BTreeMap<String, DependencySpec>,
    #[serde(default)]
    pub dev_dependencies: BTreeMap<String, DependencySpec>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct DependencyConfig {
    pub version: Option<String>,
    pub optional: Option<bool>,
    pub features: Option<Vec<String>>,
    pub default_features: Option<bool>,
    pub path: Option<String>,
}
