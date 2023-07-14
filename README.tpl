# cargo-hackerman
  ![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)
  [![cargo-hackerman on crates.io](https://img.shields.io/crates/v/cargo-hackerman)](https://crates.io/crates/cargo-hackerman)
  [![cargo-hackerman on docs.rs](https://docs.rs/cargo-hackerman/badge.svg)](https://docs.rs/cargo-hackerman)
  [![Source Code Repository](https://img.shields.io/badge/Code-On%20github.com-blue)](https://github.com/pacak/hackerman)
  [![cargo-hackerman on deps.rs](https://deps.rs/repo/github/pacak/cargo-hackerman/status.svg)](https://deps.rs/repo/github/pacak/hackerman)



# Hackerman solves following problems

- [Avoiding unnecessary recompilations](#cargo-hackerman-hack)
- [Explaining why workspace depends on a certain crate](#cargo-hackerman-explain)
- [Explaining what crates are needed for a certain crate](#cargo-hackerman-tree)
- [Finding crates that a workspace needs in multiple versions](#cargo-hackerman-dupes)
- [Quick lookup for crate documentation, homepage, etc](#cargo-hackerman-show)


[Command line summary](#command-summary)

# Feature unification, what does this mean for me as a user?

As a part of working with workspaces cargo performs feature unification:
<https://doc.rust-lang.org/cargo/reference/features.html#feature-unification>

What does this mean?

Suppose you have a workspace

```toml
[workspace]
members = [ "mega", "potato" ]
```

With two members: `mega`

```toml
[package]
name = "mega"

[dependencies]
potatoer = { version = "0.2.1", features = ["mega"] }
```

And `potato`

```toml
[package]
name = "potato"

[dependencies]
potatoer = { version = "0.2.1", features = ["potato"] }
```

Both of which depend on a common third party crate `potatoer` but with different features:
`mega` is interested in `"mega"` feature, `potato` is interested in `"potato"` one.

when running different commands you end up requiring several different versions of `potatoer`
crate.

- Whole workspace commands will use version with unified features:

  ```sh
  cargo check # this will use potatoer with both "mega" and "potato"
  ```

- Commands operating on a single crate will use versions without unification:

  ```sh
  cargo check -p mega           # this will use potatoer with "mega" feature
  cargo check -p potatoer       # this will use potatoer with "potato" feature
  cargo check -p mega -p potato # this will require both "mega" and "potato"
  ```

If a dependency with required combination is not present - cargo will compile it.

One way to avoid this problem is to make sure that if members of a workspace depend on a
crate - they depend on it with the same set of features. Maintaining it by hand is error prone
and that's when `hackerman hack` and `hackerman restore` come in.

When used with `--lock` option `hackerman` will take a checksum of all the dependencies and
will save it inside `Cargo.toml` file under `["package.metadata.hackerman.lock"]` and
subsequent calls to check will confirm that this checksum is still valid.

This is required to make sure that original (unhacked) dependencies are saved and can be
restored at a later point.

It is possible to hardcode `--lock` option in a `Cargo.toml` file that defines the workspace:

```toml
[workspace.metadata.hackerman]
lock = true
```

At the moment unification is performed for current target only and without crosscompilation
support. Automatic update for workspace toml files might not work if you are specifying
dependencies using syntax different than by version or `{}`:

```toml
potato = "3.14"               # this is okay
banana = { version = "3.14" } # this is also okay
```

### Hackerman mergetool

Resolves merge and rebase conflicts for `Cargo.toml` files changed by hackerman


To use it you want something like this

global `.gitconfig` or local `.git/config`.

```text
[merge "hackerman"]
    name = merge restored files with hackerman
    driver = cargo hackerman merge %O %A %B %P
```

`gitattributes` file, could be local per project or global

```text
Cargo.toml merge=hackerman
```

To create a global `gitattributes` file you need to specify a path to it inside the global git
config:

```text
[core]
    attributesfile = ~/.gitattributes
```

### Hackerman vs no hack vs single hack crate

Here I'm comparing effects of different approaches to unification on a workspace. Without any
changes clean check over the whole workspace that involves compiling of all the external
dependencies takes 672 seconds.

Workspace contains a bunch of crates, from which I selected crates `a`, `b`, `c`, etc, such
that crate `b` imports crate `a`, crate `c` imports crate `b`, etc. crate `a` contains no
external dependencies, other crates to.

- _no hack_ - checks are done without any hacks.
- _hackerman_ - hack was generated with `cargo hackerman hack` command and new dependencies are
  added to every crate
- _manual hack_ - hack consists of a single crate with all the crates that have different
  combinations of features and this new crate is included as a dependency to every crate in the
  workspace

Before runnining the command I clean the compilation results then commands for each column
sequentially

| command      | no hack | hackerman | manual hack |
| ------------ | ------- | --------- | ----------- |
| `check -p a` |   0.86s | 0.80s     | 215.39s     |
| `check -p b` | 211.30s | 240.15s   | 113.56s     |
| `check -p c` | 362.69s | 233.38s   | 176.73s     |
| `check -p d` | 36.16s  | 0.24s     | 0.25s       |
| `check -p e` | 385.35s | 66.34s    | 375.22s     |
| `check`      | 267.06s | 93.29s    | 81.50s      |
| total        | 1263.42 | 634.20    | 962.65      |


<USAGE>
