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





# Command summary

  * [`cargo hackerman`↴](#cargo-hackerman)
  * [`cargo hackerman hack`↴](#cargo-hackerman-hack)
  * [`cargo hackerman restore`↴](#cargo-hackerman-restore)
  * [`cargo hackerman check`↴](#cargo-hackerman-check)
  * [`cargo hackerman merge`↴](#cargo-hackerman-merge)
  * [`cargo hackerman explain`↴](#cargo-hackerman-explain)
  * [`cargo hackerman dupes`↴](#cargo-hackerman-dupes)
  * [`cargo hackerman tree`↴](#cargo-hackerman-tree)
  * [`cargo hackerman show`↴](#cargo-hackerman-show)

# cargo hackerman

A collection of tools that help your workspace to compile fast

**Usage**: **`cargo hackerman`** _`COMMAND ...`_

**Available options:**
- **`-h`**, **`--help`** &mdash; 
  Prints help information
- **`-V`**, **`--version`** &mdash; 
  Prints version information



**Available commands:**
- **`hack`** &mdash; 
  Unify crate dependencies across individual crates in the workspace
- **`restore`** &mdash; 
  Remove crate dependency unification added by the `hack` command
- **`check`** &mdash; 
  Check if unification is required and if checksums are correct
- **`merge`** &mdash; 
  Restore files and merge with the default merge driver
- **`explain`** &mdash; 
  Explain why some dependency is present. Both feature and version are optional
- **`dupes`** &mdash; 
  Lists all the duplicates in the workspace
- **`tree`** &mdash; 
  Make a tree out of dependencies
- **`show`** &mdash; 
  Show crate manifest, readme, repository or documentation



You can pass **`--help`** twice for more detailed help


# cargo hackerman hack

Unify crate dependencies across individual crates in the workspace

**Usage**: **`cargo hackerman`** **`hack`** _`CARGO_OPTS`_ \[**`--dry`**\] \[**`--lock`**\] \[**`-D`**\]

You can undo those changes using `cargo hackerman restore`.

**Cargo options:**
- **`    --manifest-path`**=_`PATH`_ &mdash; 
  Path to Cargo.toml file
- **`    --frozen`** &mdash; 
  Require Cargo.lock and cache are up to date
- **`    --locked`** &mdash; 
  Require Cargo.lock is up to date
- **`    --offline`** &mdash; 
  Run without accessing the network
- **`-v`**, **`--verbose`** &mdash; 
  increase verbosity, can be used several times



**Available options:**
- **`    --dry`** &mdash; 
  Don't perform action, only display it
- **`    --lock`** &mdash; 
  Include dependencies checksum into stash

  This helps to ensure you can go back to original (unhacked) dependencies: to be able to restore the original dependencies hackerman needs to have them stashed in `Cargo.toml` file. If CI detects checksum mismatch this means dependencies were updated on hacked sources. You should instead restore them, update and hack again.

  You can make locking the default behavior by adding this to `Cargo.toml` in the workspace

  
  ```text
  [workspace.metadata.hackerman]
  lock = true
  ```

- **`-D`**, **`--no-dev`** &mdash; 
  Don't unify dev dependencies
- **`-h`**, **`--help`** &mdash; 
  Prints help information



`cargo-hackerman hack` calculates and adds a minimal set of extra dependencies to all the workspace members such that features of all the dependencies of this crate stay the same when it is used as part of the whole workspace or by itself.

Once dependencies are hacked you should restore them before making any changes.


# cargo hackerman restore

Remove crate dependency unification added by the `hack` command

**Usage**: **`cargo hackerman`** **`restore`** _`CARGO_OPTS`_ \[_`TOML`_\]...

**Cargo options:**
- **`    --manifest-path`**=_`PATH`_ &mdash; 
  Path to Cargo.toml file
- **`    --frozen`** &mdash; 
  Require Cargo.lock and cache are up to date
- **`    --locked`** &mdash; 
  Require Cargo.lock is up to date
- **`    --offline`** &mdash; 
  Run without accessing the network
- **`-v`**, **`--verbose`** &mdash; 
  increase verbosity, can be used several times



**Available positional items:**
- _`TOML`_ &mdash; 
  Restore individual files instead of the whole workspace



**Available options:**
- **`-h`**, **`--help`** &mdash; 
  Prints help information


# cargo hackerman check

Check if unification is required and if checksums are correct

Similar to `cargo-hackerman hack --dry`, but also sets exit status to 1 so you can use it as part of CI process

**Usage**: **`cargo hackerman`** **`check`** _`CARGO_OPTS`_ \[**`-D`**\]

**Cargo options:**
- **`    --manifest-path`**=_`PATH`_ &mdash; 
  Path to Cargo.toml file
- **`    --frozen`** &mdash; 
  Require Cargo.lock and cache are up to date
- **`    --locked`** &mdash; 
  Require Cargo.lock is up to date
- **`    --offline`** &mdash; 
  Run without accessing the network
- **`-v`**, **`--verbose`** &mdash; 
  increase verbosity, can be used several times



**Available options:**
- **`-D`**, **`--no-dev`** &mdash; 
  Don't unify dev dependencies
- **`-h`**, **`--help`** &mdash; 
  Prints help information


# cargo hackerman merge

Restore files and merge with the default merge driver

**Usage**: **`cargo hackerman`** **`merge`** _`BASE`_ _`LOCAL`_ _`REMOTE`_ _`RESULT`_

**Available options:**
- **`-h`**, **`--help`** &mdash; 
  Prints help information



To use it you would add something like this to `~/.gitconfig` or `.git/config`

  ```text
  [merge "hackerman"]
  name = merge restored files with hackerman
  driver = cargo hackerman merge %O %A %B %P
  ```


And something like this to `.git/gitattributes`

  ```text
  Cargo.toml merge=hackerman
  ```

# cargo hackerman explain

Explain why some dependency is present. Both feature and version are optional

**Usage**: **`cargo hackerman`** **`explain`** _`CARGO_OPTS`_ \[**`-T`**\] \[**`-P`**\] \[**`-s`**\] _`CRATE`_ \[_`FEATURE`_\] \[_`VERSION`_\]

**Cargo options:**
- **`    --manifest-path`**=_`PATH`_ &mdash; 
  Path to Cargo.toml file
- **`    --frozen`** &mdash; 
  Require Cargo.lock and cache are up to date
- **`    --locked`** &mdash; 
  Require Cargo.lock is up to date
- **`    --offline`** &mdash; 
  Run without accessing the network
- **`-v`**, **`--verbose`** &mdash; 
  increase verbosity, can be used several times



**Available options:**
- **`-T`**, **`--no-transitive-opt`** &mdash; 
  Don't strip redundant links
- **`-P`**, **`--package-nodes`** &mdash; 
  Use package nodes instead of feature nodes
- **`-s`**, **`--stdout`** &mdash; 
  Print dot file to stdout instead of spawning `xdot`
- **`-h`**, **`--help`** &mdash; 
  Prints help information



 With large amount of dependencies it might be difficult to tell why exactly some sub-sub-sub dependency is included. hackerman explain solves this problem by tracing the dependency chain from the target and to the workspace.

`explain` starts at a given crate/feature and follows reverse dependency links until it reaches all the crossing points with the workspace but without entering the workspace itself.

White nodes represent workspace members, round nodes represent features, octagonal nodes represent base crates. Dotted line represents dev-only dependency, dashed line - both dev and normal but with different features across them. Target is usually highlighted. By default hackerman expands packages info feature nodes which can be reverted with `-P` and tries to reduce transitive dependencies to keep the tree more readable - this can be reverted with `-T`.

If a crate is present in several versions you can specify version of the one you are interested in but it's optional.

You can also specify which feature to look for, otherwise hackerman will be looking for all of them.


# cargo hackerman dupes

Lists all the duplicates in the workspace

**Usage**: **`cargo hackerman`** **`dupes`** _`CARGO_OPTS`_

**Cargo options:**
- **`    --manifest-path`**=_`PATH`_ &mdash; 
  Path to Cargo.toml file
- **`    --frozen`** &mdash; 
  Require Cargo.lock and cache are up to date
- **`    --locked`** &mdash; 
  Require Cargo.lock is up to date
- **`    --offline`** &mdash; 
  Run without accessing the network
- **`-v`**, **`--verbose`** &mdash; 
  increase verbosity, can be used several times



**Available options:**
- **`-h`**, **`--help`** &mdash; 
  Prints help information


# cargo hackerman tree

Make a tree out of dependencies

**Usage**: **`cargo hackerman`** **`tree`** _`CARGO_OPTS`_ \[**`-T`**\] \[**`-D`**\] \[**`-P`**\] \[**`-w`**\] \[**`-s`**\] \[_`CRATE`_\] \[_`FEATURE`_\] \[_`VERSION`_\]

**Cargo options:**
- **`    --manifest-path`**=_`PATH`_ &mdash; 
  Path to Cargo.toml file
- **`    --frozen`** &mdash; 
  Require Cargo.lock and cache are up to date
- **`    --locked`** &mdash; 
  Require Cargo.lock is up to date
- **`    --offline`** &mdash; 
  Run without accessing the network
- **`-v`**, **`--verbose`** &mdash; 
  increase verbosity, can be used several times



**Available options:**
- **`-T`**, **`--no-transitive-opt`** &mdash; 
  Don't strip redundant links
- **`-D`**, **`--no-dev`** &mdash; 
  Don't include dev dependencies
- **`-P`**, **`--package-nodes`** &mdash; 
  Use package nodes instead of feature nodes
- **`-w`**, **`--workspace`** &mdash; 
  Keep within the workspace
- **`-s`**, **`--stdout`** &mdash; 
  Print dot file to stdout instead of spawning `xdot`
- **`-h`**, **`--help`** &mdash; 
  Prints help information



Examples:

  ```sh
  cargo hackerman tree rand 0.8.4
  cargo hackerman tree serde_json preserve_order
  ```

# cargo hackerman show

Show crate manifest, readme, repository or documentation

**Usage**: **`cargo hackerman`** **`show`** _`CARGO_OPTS`_ \[**`-m`** | **`-r`** | **`-d`** | **`-R`**\] _`CRATE`_ \[_`VERSION`_\]

**Cargo options:**
- **`    --manifest-path`**=_`PATH`_ &mdash; 
  Path to Cargo.toml file
- **`    --frozen`** &mdash; 
  Require Cargo.lock and cache are up to date
- **`    --locked`** &mdash; 
  Require Cargo.lock is up to date
- **`    --offline`** &mdash; 
  Run without accessing the network
- **`-v`**, **`--verbose`** &mdash; 
  increase verbosity, can be used several times



**Available options:**
- **`-m`**, **`--manifest`** &mdash; 
  Show crate manifest
- **`-r`**, **`--readme`** &mdash; 
  Show crate readme
- **`-d`**, **`--doc`** &mdash; 
  Open documentation URL
- **`-R`**, **`--repository`** &mdash; 
  Repository
- **`-h`**, **`--help`** &mdash; 
  Prints help information



Examples:

  ```sh
  cargo hackerman show --repository syn
  ```


