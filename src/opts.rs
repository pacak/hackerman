use std::{ffi::OsString, path::PathBuf, str::FromStr};

use bpaf::{positional_if, short, Bpaf, Parser};
use cargo_metadata::{Metadata, Version};
use tracing::Level;

#[derive(Debug, Clone, Bpaf)]
#[bpaf(options("hackerman"), version)]
pub enum Action {
    #[bpaf(command)]
    /// Unify crate dependencies across individual crates in the workspace
    Hack {
        #[bpaf(external(profile))]
        profile: Profile,
        /// don't perform action, only display it
        dry: bool,
        /// Include dependencies checksum into stash
        lock: bool,
        /// Don't unify dev dependencies
        no_dev: bool,
    },

    /// Remove crate dependency unification added by the 'hack' command
    #[bpaf(command)]
    Restore {
        #[bpaf(external(profile))]
        profile: Profile,
        /// Restore single file instead of the whole workspace
        #[bpaf(positional_os("TOML"))]
        single: Option<PathBuf>,
    },

    /// Check if unification is required and if checksums are correct
    #[bpaf(command)]
    Check {
        #[bpaf(external(profile))]
        profile: Profile,
        /// Don't unify dev dependencies
        no_dev: bool,
    },

    /// Restore files and merge with the default merge driver
    #[bpaf(command("merge"))]
    MergeDriver {
        #[bpaf(positional("BASE"))]
        base: PathBuf,
        #[bpaf(positional("LOCAL"))]
        local: PathBuf,
        #[bpaf(positional("REMOTE"))]
        remote: PathBuf,
        #[bpaf(positional("RESULT"))]
        result: PathBuf,
    },

    #[bpaf(command)]
    /// Explain why some dependency is present. Both feature and version are optional
    Explain {
        #[bpaf(external(profile))]
        profile: Profile,

        /// Don't strip redundant links
        #[bpaf(short('T'), long)]
        no_transitive_opt: bool,

        /// Use package nodes instead of feature nodes
        #[bpaf(short('P'), long)]
        package_nodes: bool,

        #[bpaf(positional("CRATE"))]
        krate: String,
        #[bpaf(external(feature_if))]
        feature: Option<String>,
        #[bpaf(external(version_if))]
        version: Option<Version>,
    },

    /// Lists all the duplicates in the workspace
    #[bpaf(command)]
    Dupes {
        #[bpaf(external(profile))]
        profile: Profile,
    },

    #[bpaf(command)]
    /// Make a tree out of dependencies
    Tree {
        #[bpaf(external(profile))]
        profile: Profile,

        /// Don't strip redundant links
        #[bpaf(short('T'), long)]
        no_transitive_opt: bool,

        /// Don't include dev dependencies
        #[bpaf(short('D'), long)]
        no_dev: bool,

        /// Use package nodes instead of feature nodes
        #[bpaf(short('P'), long)]
        package_nodes: bool,

        /// Keep within the workspace
        #[bpaf(short, long)]
        workspace: bool,

        #[bpaf(positional("CRATE"))]
        krate: Option<String>,
        #[bpaf(external(feature_if))]
        feature: Option<String>,
        #[bpaf(external(version_if))]
        version: Option<Version>,
    },

    #[bpaf(command("show"))]
    /// Show info about a crate
    ShowCrate {
        #[bpaf(external(profile))]
        profile: Profile,
        #[bpaf(external(focus), fallback(Focus::Manifest))]
        focus: Focus,
        #[bpaf(positional("CRATE"))]
        krate: String,
        #[bpaf(external(version_if))]
        version: Option<Version>,
    },
}

fn feature_if() -> Parser<Option<String>> {
    positional_if("FEATURE", |v| !is_version(v))
}

fn version_if() -> Parser<Option<Version>> {
    positional_if("VERSION", is_version).map(|s| s.map(|v| Version::from_str(&v).unwrap()))
}

#[derive(Debug, Clone, Bpaf)]
pub struct Profile {
    #[bpaf(argument_os("PATH"), fallback(profile_fallback()))]
    /// Path to Cargo.toml file, defaults to one in the current directory
    pub manifest_path: PathBuf,

    /// Require Cargo.lock and cache are up to date
    pub frozen: bool,
    /// Require Cargo.lock is up to date
    pub locked: bool,
    /// Run without accessing the network
    pub offline: bool,

    #[bpaf(external)]
    pub verbosity: Level,
}

impl Profile {
    pub fn exec(&self) -> anyhow::Result<Metadata> {
        let mut cmd = cargo_metadata::MetadataCommand::new();

        let mut extra = Vec::new();
        if self.frozen {
            extra.push(String::from("--frozen"));
        }
        if self.locked {
            extra.push(String::from("--locked"));
        }
        if self.offline {
            extra.push(String::from("--offline"));
        }
        cmd.manifest_path(&self.manifest_path);
        cmd.other_options(extra);

        Ok(cmd.exec()?)
    }
}

fn profile_fallback() -> PathBuf {
    "Cargo.toml".into()
}

#[derive(Debug, Clone)]
pub enum Command {
    Explain(Explain),
    Hack(Hack),
    Restore(Option<OsString>),
    Duplicates,
    Verify,
    WorkspaceTree,
    PackageTree(String, Option<String>, Option<String>),
    ShowPackage(String, Option<String>, Option<Focus>),
    Mergedriver(OsString, OsString, OsString, OsString),
}

#[derive(Debug, Clone, Bpaf)]
pub enum Focus {
    #[bpaf(short, long)]
    /// Show crate manifest
    Manifest,
    #[bpaf(short, long)]
    /// Show crate readme
    Readme,
    #[bpaf(short, long("doc"))]
    /// Open documentation URL
    Documentation,
}

#[derive(Debug, Clone)]
pub struct Explain {
    pub krate: String,
    pub feature: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Hack {
    pub dry: bool,
    pub lock: bool,
}

fn is_version(v: &str) -> bool {
    v == "*" || semver::Version::parse(v).is_ok()
}

fn verbosity() -> Parser<Level> {
    short('v')
        .help("increase verbosity, can be used several times")
        .req_flag(())
        .many()
        .map(|xs| match xs.len() {
            0 => Level::WARN,
            1 => Level::INFO,
            2 => Level::DEBUG,
            _ => Level::TRACE,
        })
}
