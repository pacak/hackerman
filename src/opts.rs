use bpaf::{doc::Style, positional, short, Bpaf, Parser};
use cargo_metadata::Metadata;
use semver::Version;
use std::{path::PathBuf, str::FromStr};
use tracing::Level;

const DETAILED_HELP: &[(&str, Style)] = &[
    ("You can pass ", Style::Text),
    ("--help", Style::Literal),
    (" twice for more detailed help", Style::Text),
];

#[derive(Debug, Clone, Bpaf)]
#[bpaf(options("hackerman"), version, footer(DETAILED_HELP))]
/// A collection of tools that help your workspace to compile fast
pub enum Action {
    #[bpaf(command)]
    /// Unify crate dependencies across individual crates in the workspace
    ///
    ///
    /// You can undo those changes using `cargo hackerman restore`.
    ///
    ///
    /// `cargo-hackerman hack` calculates and adds a minimal set of extra dependencies
    /// to all the workspace members such that features of all the dependencies
    /// of this crate stay the same when it is used as part of the whole workspace
    /// or by itself.
    ///
    /// Once dependencies are hacked you should restore them before making any
    /// changes.
    Hack {
        #[bpaf(external(profile))]
        profile: Profile,

        /// Don't perform action, only display it
        dry: bool,

        /// Include dependencies checksum into stash
        ///
        /// This helps to ensure you can go back to original (unhacked) dependencies: to be able to
        /// restore the original dependencies hackerman needs to have them stashed in `Cargo.toml`
        /// file. If CI detects checksum mismatch this means dependencies were updated on hacked
        /// sources. You should instead restore them, update and hack again.
        ///
        /// You can make locking the default behavior by adding this to `Cargo.toml` in the
        /// workspace
        ///
        /// ```text
        /// [workspace.metadata.hackerman]
        /// lock = true
        /// ```
        ///
        lock: bool,

        /// Don't unify dev dependencies
        #[bpaf(short('D'), long)]
        no_dev: bool,
    },

    /// Remove crate dependency unification added by the `hack` command
    #[bpaf(command)]
    Restore {
        #[bpaf(external(profile))]
        profile: Profile,

        /// Restore individual files instead of the whole workspace
        #[bpaf(positional("TOML"))]
        separate: Vec<PathBuf>,
    },

    /// Check if unification is required and if checksums are correct
    ///
    /// Similar to `cargo-hackerman hack --dry`, but also sets exit status to 1
    /// so you can use it as part of CI process
    #[bpaf(command)]
    Check {
        #[bpaf(external(profile))]
        profile: Profile,

        /// Don't unify dev dependencies
        #[bpaf(short('D'), long)]
        no_dev: bool,
    },

    /// Restore files and merge with the default merge driver
    ///
    ///
    ///
    ///
    /// To use it you would add something like this to `~/.gitconfig` or `.git/config`
    ///
    /// ```text
    /// [merge "hackerman"]
    /// name = merge restored files with hackerman
    /// driver = cargo hackerman merge %O %A %B %P
    /// ```
    ///
    /// And something like this to `.git/gitattributes`
    ///
    /// ```text
    /// Cargo.toml merge=hackerman
    /// ```
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
    ///
    ///
    ///
    ///
    ///
    /// With large amount of dependencies it might be difficult to tell why exactly some
    /// sub-sub-sub dependency is included. hackerman explain solves this problem by tracing
    /// the dependency chain from the target and to the workspace.
    ///
    /// `explain` starts at a given crate/feature and follows reverse dependency links until it
    /// reaches all the crossing points with the workspace but without entering the workspace
    /// itself.
    ///
    /// White nodes represent workspace members, round nodes represent features, octagonal nodes
    /// represent base crates. Dotted line represents dev-only dependency, dashed line - both
    /// dev and normal but with different features across them. Target is usually highlighted.
    /// By default hackerman expands packages info feature nodes which can be reverted with
    /// `-P` and tries to reduce transitive dependencies to keep the tree more readable -
    /// this can be reverted with `-T`.
    ///
    /// If a crate is present in several versions you can specify version of the one you
    /// are interested in but it's optional.
    ///
    /// You can also specify which feature to look for, otherwise hackerman will be
    /// looking for all of them.
    Explain {
        #[bpaf(external(profile))]
        profile: Profile,

        /// Don't strip redundant links
        #[bpaf(short('T'), long)]
        no_transitive_opt: bool,

        /// Use package nodes instead of feature nodes
        #[bpaf(short('P'), long)]
        package_nodes: bool,

        /// Print dot file to stdout instead of spawning `xdot`
        #[bpaf(short, long)]
        stdout: bool,

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
    ///
    ///
    ///
    ///
    /// Examples:
    ///
    /// ```sh
    /// cargo hackerman tree rand 0.8.4
    /// cargo hackerman tree serde_json preserve_order
    /// ```
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

        /// Print dot file to stdout instead of spawning `xdot`
        #[bpaf(short, long)]
        stdout: bool,

        #[bpaf(positional("CRATE"))]
        krate: Option<String>,
        #[bpaf(external(feature_if))]
        feature: Option<String>,
        #[bpaf(external(version_if))]
        version: Option<Version>,
    },

    #[bpaf(command("show"))]
    /// Show crate manifest, readme, repository or documentation
    ///
    ///
    ///
    ///
    /// Examples:
    ///
    /// ```sh
    /// cargo hackerman show --repository syn
    /// ```
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

fn feature_if() -> impl Parser<Option<String>> {
    positional::<String>("FEATURE")
        .parse::<_, _, &'static str>(|s| match Version::from_str(&s) {
            Err(_) => Ok(s),
            Ok(_) => Err("not a feature"),
        })
        .optional()
        .catch()
}

fn version_if() -> impl Parser<Option<Version>> {
    positional::<Version>("VERSION").optional().catch()
}

#[derive(Debug, Clone, Bpaf)]
/// Cargo options:
#[bpaf(custom_usage(&[("CARGO_OPTS", Style::Metavar)]))]
pub struct Profile {
    #[bpaf(argument("PATH"), fallback("Cargo.toml".into()))]
    /// Path to Cargo.toml file
    pub manifest_path: PathBuf,

    /// Require Cargo.lock and cache are up to date
    pub frozen: bool,
    /// Require Cargo.lock is up to date
    pub locked: bool,
    /// Run without accessing the network
    pub offline: bool,

    #[bpaf(external)]
    pub verbosity: (usize, Level),
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
        for _ in 0..self.verbosity.0 {
            extra.push(String::from("-v"));
        }
        cmd.manifest_path(&self.manifest_path);
        cmd.other_options(extra);

        Ok(cmd.exec()?)
    }
}

#[derive(Debug, Clone, Bpaf)]
pub enum Focus {
    #[bpaf(short, long)]
    /// Show crate manifest
    Manifest,

    #[bpaf(short, long)]
    /// Show crate readme
    Readme,

    #[bpaf(short, long("doc"), long("docs"))]
    /// Open documentation URL
    Documentation,

    #[bpaf(short('R'), long, long("repo"), long("git"))]
    /// Repository
    Repository,
}

fn verbosity() -> impl Parser<(usize, Level)> {
    short('v')
        .long("verbose")
        .help("increase verbosity, can be used several times")
        .req_flag(())
        .count()
        .map(|x| {
            (
                x,
                match x {
                    0 => Level::WARN,
                    1 => Level::INFO,
                    2 => Level::DEBUG,
                    _ => Level::TRACE,
                },
            )
        })
}

#[cfg(all(test, unix))]
mod readme {

    fn write_updated(new_val: &str, path: impl AsRef<std::path::Path>) -> std::io::Result<bool> {
        use std::io::Read;
        use std::io::Seek;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .read(true)
            .create(true)
            .open(path)?;
        let mut current_val = String::new();
        file.read_to_string(&mut current_val)?;
        if current_val != new_val {
            file.set_len(0)?;
            file.seek(std::io::SeekFrom::Start(0))?;
            std::io::Write::write_all(&mut file, new_val.as_bytes())?;
            Ok(false)
        } else {
            Ok(true)
        }
    }

    #[test]
    fn docs_are_up_to_date() {
        let usage = super::action().render_markdown("cargo hackerman");
        let readme = std::fs::read_to_string("README.tpl").unwrap();
        let docs = readme.replacen("<USAGE>", &usage, 1);
        assert!(write_updated(&docs, "README.md").unwrap());
    }
}
