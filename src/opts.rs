use std::ffi::OsString;

use bpaf::*;
use tracing::Level;

#[derive(Debug, Clone)]
pub enum Command {
    Explain(Explain),
    Hack(Hack),
    Restore(Restore),
    Verify,
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
}

#[derive(Debug, Clone)]
pub struct Restore {
    pub dry: bool,
}

fn explain() -> Parser<Explain> {
    let krate = positional("CRATE");
    let feature = positional_if("FEATURE", |v| semver::Version::parse(v).is_err());
    let version = positional_if("VERSION", |v| semver::Version::parse(v).is_ok());
    construct!(Explain {
        krate,
        feature,
        version,
    })
}

fn explain_cmd() -> Parser<Command> {
    let info = Info::default()
        .descr("Explain why a certain crate or a feature is included in the workspace")
        .header(
            "\
If a crate is present in several versions you need to specify the
version of one you are interested in, otherwise it's optional.

Examples:

    cargo hackerman explain rand 0.8.4
    cargo hackerman explain serde_json preserve_order",
        )
        .for_parser(explain());

    command(
        "explain",
        Some("Explain a dependency in the current workspace"),
        info,
    )
    .map(Command::Explain)
}

fn hack_cmd() -> Parser<Command> {
    let dry = dry_run();
    let info = Info::default()
        .descr("Unify crate dependencies across individual crates in the workspace")
        .for_parser(construct!(Hack { dry }));
    command("hack", Some("Unify crate dependencies"), info).map(Command::Hack)
}

fn restore_cmd() -> Parser<Command> {
    let dry = dry_run();
    let info = Info::default()
        .descr("Remove crate dependency unification added by the 'hack' command")
        .for_parser(construct!(Restore { dry }));
    command("restore", Some("Remove unification"), info).map(Command::Restore)
}

fn verify_cmd() -> Parser<Command> {
    let info = Info::default()
        .descr("Check if unification is required and other invariants")
        .for_parser(Parser::pure(()));
    command(
        "check",
        Some("Check for unification and other issues"),
        info,
    )
    .map(|_| Command::Verify)
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

fn dry_run() -> Parser<bool> {
    short('d')
        .long("dry")
        .help("report actions to be performed without actually performing them")
        .switch()
}

pub fn options() -> OptionParser<(Level, OsString, Command)> {
    Info::default().for_parser(command(
        "hackerman",
        Some("A set of commands to do strange things to the workspace"),
        options_inner(),
    ))
}

fn custom_manifest() -> Parser<OsString> {
    long("manifest-path")
        .help("Path to Cargo.toml")
        .argument_os("PATH")
        .fallback("Cargo.toml".into())
}

// For reasons (?) cargo doesn't replace the command line used so we need to put a command inside a
// command.
fn options_inner() -> OptionParser<(Level, OsString, Command)> {
    let v = verbosity();
    let cmd = explain_cmd()
        .or_else(hack_cmd())
        .or_else(restore_cmd())
        .or_else(verify_cmd());
    let custom_manifest = custom_manifest();
    let opts = tuple!(v, custom_manifest, cmd);
    Info::default().for_parser(opts)
}