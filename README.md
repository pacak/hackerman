Hackerman solves following problems

- [Avoiding unnecessary recompilations](#hackerman-hack--check--restore)
- [Explaining why workspace depends on a certain crate](#hackerman-explain)
- [Explaining what crates are needed for a certain crate](#hackerman-tree)
- [Finding crates that a workspace needs in multiple versions](#hackerman-dupes)
- [Quick lookup for crate documentation, homepage, etc](#hackerman-show)


Currently included functionality contains

### Hackerman hack / check / restore

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
both both of which depend on a common third party crate `potatoer` but with different features:
`mega` is interested in `"mega"` aspect, `potato` is interested in `"potato"` one.

when running different commands you end up requiring several different versions of `potatoer`
crate.

- Whole workspace commands will use version with unified features:
  ```bash
  cargo check # this will use potatoer with both "mega" and "potato"
  ```
- Commands operating on a single crate will use versions without unification:
  ```bash
  cargo check -p mega           # this will use potatoer with "mega" feature
  cargo check -p potatoer       # this will use potatoer with "potato" feature
  cargo check -p mega -p potato # this will require both "mega" and "potato"
  ```
If a dependency with required combination is not present - cargo will compile it.

One way to avoid this problem is to make sure that if members of a workspace depend on some
crate - they depend on it with the same set of features. Maintaining it by hand is error prone
and there's when `hackerman hack` and `hackerman restore` comes in.

When used with `--lock` option will take a checksum of all the dependencies and will
save it inside Cargo.toml file under ["package.metadata.hackerman.lock"] and subsequent
calls to check will confirm that this checksum is still valid.

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


### Hackerman explain

With large amount of dependencies it might be difficult to tell why exactly some sub-sub-sub
dependency is included. `hackerman explain` solves this problem:

Explain starts at a given crate/feature and follows reverse dependencies links
until reaches all the crossing points with the workspace but without entering the workspace itself.

White nodes represent workspace members, round nodes represent features, octagonal nodes
represent base crates. Dotted line represents dev-only dependency, dashed line - both dev and normal but
with different features across them. Target is usually highlighted. By default hackerman
expands packages info feature nodes which can be reverted with `-P` and tries to reduce
transitive dependencies to keep the tree more readable - this can be reverted with `-T`.

If a crate is present in several versions you can specify the
version of one you are interested in but it's optional.

You can also specify which feature to look for, otherwise hackerman
will be looking for all of them.

Note, `hackerman` uses `xdot` by default. If it's not available - it is possible to install
`hackerman` without `"spawn_xdot"` feature to produce `.dot` file to stdout

Examples:

```text
cargo hackerman explain rand 0.8.4
cargo hackerman explain serde_json preserve_order
```

### Hackerman tree

One different problem is figuring out what some crate require. Welcome `hackerman
tree`:

Tree starts at a given crate/feature and follows dependencies links all the way to the end.

If a crate is present in several versions you need to specify the
version of one you are interested in, otherwise it's optional.

You can also specify which feature to look for, otherwise hackerman
will be looking for all of them.

Examples:

```text
cargo hackerman tree rand 0.8.4
cargo hackerman explain serde_json preserve_order
```

### Hackerman dupes

`cargo hackerman dupes` lists all the packages used in workspace dependencies present in more
than one version:

```text
cargo hackerman dupes
```


### Hackerman show

`cargo hackerman show` solves a problem of getting more information about the dependency.
Currently it can provide following information:

- crate manifest file for exact version used
- crate documentation - for _exact_ version used
- crate repository if specified

### Hackerman mergetool

Resolves merge and rebase conflicts for Cargo.toml files changed by hackerman

To use it you want something like this

global `.gitconfig` or local `.git/config`.
```text
[merge "hackerman"]
    name = merge restored files with hackerman
    driver = cargo hackerman merge %O %A %B %P
```

gitattributes file, could be local per project or global
```text
Cargo.toml merge=hackerman
```

To create a global gitattributes file you need to specify a path to it inside the global git
config:
```text
[core]
    attributesfile = ~/.gitattributes
```
