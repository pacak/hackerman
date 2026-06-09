use std::path::{Path, PathBuf};

use crate::config::{CrateConfig, DependencySpec, WorkspaceConfig};

pub struct GeneratedWorkspace {
    pub root: PathBuf,
    pub files: Vec<(PathBuf, String)>,
}

pub fn generate(config: &WorkspaceConfig, output_base: &Path) -> GeneratedWorkspace {
    let ws_name = &config.workspace.name;
    let root = output_base.join(ws_name);

    let mut files = Vec::new();

    // Generate workspace Cargo.toml
    let members: Vec<String> = config
        .crates
        .iter()
        .filter(|c| c.workspace_member)
        .map(|c| format!("\"{}-{}\"", c.name, c.version))
        .collect();

    let excludes: Vec<String> = config
        .crates
        .iter()
        .filter(|c| !c.workspace_member)
        .map(|c| format!("\"{}-{}\"", c.name, c.version))
        .collect();

    let ws_toml = if excludes.is_empty() {
        format!(
            "[workspace]\nresolver = \"2\"\nmembers = [{}]\n",
            members.join(", ")
        )
    } else {
        format!(
            "[workspace]\nresolver = \"2\"\nmembers = [{}]\nexclude = [{}]\n",
            members.join(", "),
            excludes.join(", ")
        )
    };

    files.push((root.join("Cargo.toml"), ws_toml));

    // Generate each crate
    for crate_config in &config.crates {
        let crate_dir = root.join(format!("{}-{}", crate_config.name, crate_config.version));

        let cargo_toml = generate_crate_toml(crate_config);
        files.push((crate_dir.join("Cargo.toml"), cargo_toml));

        let lib_rs = if crate_config.proc_macro {
            PROC_MACRO_LIB_RS.to_string()
        } else {
            LIB_RS.to_string()
        };
        files.push((crate_dir.join("src/lib.rs"), lib_rs));
    }

    GeneratedWorkspace { root, files }
}

fn generate_crate_toml(config: &CrateConfig) -> String {
    let mut out = String::new();

    out.push_str("[package]\n");
    out.push_str(&format!("name = \"{}\"\n", config.name));
    out.push_str(&format!("version = \"{}\"\n", config.version));
    out.push_str("edition = \"2024\"\n\n");

    // Proc-macro crate type
    if config.proc_macro {
        out.push_str("[lib]\n");
        out.push_str("proc-macro = true\n\n");
    }

    // Dependencies
    if !config.dependencies.is_empty() {
        out.push_str("[dependencies]\n");
        for (dep_name, dep_config) in &config.dependencies {
            out.push_str(&generate_dep_line(dep_name, dep_config));
        }
        out.push('\n');
    }

    // Build dependencies
    if !config.build_dependencies.is_empty() {
        out.push_str("[build-dependencies]\n");
        for (dep_name, dep_config) in &config.build_dependencies {
            out.push_str(&generate_dep_line(dep_name, dep_config));
        }
        out.push('\n');
    }

    // Dev dependencies
    if !config.dev_dependencies.is_empty() {
        out.push_str("[dev-dependencies]\n");
        for (dep_name, dep_config) in &config.dev_dependencies {
            out.push_str(&generate_dep_line(dep_name, dep_config));
        }
        out.push('\n');
    }

    // Platform-specific dependencies
    for platform in &config.platform {
        if !platform.dependencies.is_empty() {
            out.push_str(&format!("[target.'{}'.dependencies]\n", platform.target));
            for (dep_name, dep_config) in &platform.dependencies {
                out.push_str(&generate_dep_line(dep_name, dep_config));
            }
            out.push('\n');
        }

        if !platform.build_dependencies.is_empty() {
            out.push_str(&format!(
                "[target.'{}'.build-dependencies]\n",
                platform.target
            ));
            for (dep_name, dep_config) in &platform.build_dependencies {
                out.push_str(&generate_dep_line(dep_name, dep_config));
            }
            out.push('\n');
        }

        if !platform.dev_dependencies.is_empty() {
            out.push_str(&format!(
                "[target.'{}'.dev-dependencies]\n",
                platform.target
            ));
            for (dep_name, dep_config) in &platform.dev_dependencies {
                out.push_str(&generate_dep_line(dep_name, dep_config));
            }
            out.push('\n');
        }
    }

    // Features
    if !config.features.is_empty() {
        out.push_str("[features]\n");
        for (feat_name, feat_deps) in &config.features {
            if feat_deps.is_empty() {
                out.push_str(&format!("{} = []\n", feat_name));
            } else {
                out.push_str(&format!(
                    "{} = [{}]\n",
                    feat_name,
                    format_deps_list(feat_deps)
                ));
            }
        }
    }

    out
}

fn generate_dep_line(dep_name: &str, spec: &DependencySpec) -> String {
    let config = spec.to_config();
    let mut parts = Vec::new();

    // Determine path
    let path = if let Some(ref p) = config.path {
        p.clone()
    } else {
        let dep_version = config.version.as_deref().unwrap_or("0.1.0");
        format!("../{}-{}", dep_name, dep_version)
    };
    parts.push(format!("path = \"{}\"", path));

    if let Some(optional) = config.optional
        && optional
    {
        parts.push("optional = true".to_string());
    }

    if let Some(default_features) = config.default_features
        && !default_features
    {
        parts.push("default-features = false".to_string());
    }

    if let Some(ref features) = config.features {
        let feat_str: Vec<String> = features.iter().map(|f| format!("\"{}\"", f)).collect();
        parts.push(format!("features = [{}]", feat_str.join(", ")));
    }

    format!("{} = {{ {} }}\n", dep_name, parts.join(", "))
}

fn format_deps_list(deps: &[String]) -> String {
    deps.iter()
        .map(|d| format!("\"{}\"", d))
        .collect::<Vec<_>>()
        .join(", ")
}

const LIB_RS: &str = "#[cfg(test)] mod tests { #[test] fn it_works() { assert!(true); } }";

const PROC_MACRO_LIB_RS: &str = "extern crate proc_macro; use proc_macro::TokenStream; #[proc_macro] pub fn identity(input: TokenStream) -> TokenStream { input } ";
