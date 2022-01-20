Hackerman solves following problems

- [Avoiding unnecessary recompilations](#hackerman-hack)
- [Explaining why workspace depends on a certain crate](#hackerman-explain)
- [Explaining what crates are needed for a certain crate](#hackerman-tree)
- [Finding crates that a workspace needs in multiple versions](#hackerman-dupes)
- [Hack status check](#hackerman-check)
- [Quick lookup for crate documentation, homepage, etc](#hackerman-show)


Currently included functionality contains

### Hackerman hack

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
`mega` is interested in `"mega"` aspect, `potato" is interested in `"potato"` one.

when running different commands you end up requiring several different versions of `potatoer`.

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
If a dependency with required combination is not present - cargo will compile compile it.

One way to avoid this problem is to make sure that if members of a workspace depend on some
crate - they depend on it with the same set of features. Maintaining it by hand is error prone
and there's when `hackerman hack` and `hackerman restore` comes in.

When used with --lock option will take a checksum of all the dependencies and will
save it inside Cargo.toml file under ["package.metadata.hackerman.lock"] and subsequent
calls to check will confirm that this checksum is still valid.

This is required to make sure that original (unhacked) dependencies are saved and can be
restored at a later point.

### Hackerman explain

With large amount of dependencies it might be difficult to tell why exactly some sub-sub-sub
dependency is included. `hackerman explain` solves this problem:

Explain starts at a given crate/feature and follows reverse dependencies links
until reaches all the crossing points with the workspace but without entering the workspace itself.

Red nodes represent workspace members, green nodes represent starting features.
Dotted line represents dev-only dependency, dashed line - both dev and normal but
with different features across them.

If a crate is present in several versions you can specify the
version of one you are interested in but it's optional.

You can also specify which feature to look for, otherwise hackerman
will be looking for all of them.

Examples:

    cargo hackerman explain rand 0.8.4
    cargo hackerman explain serde_json preserve_order

### Hackerman tree

One different problem is figuring out what some crate requires for working. Welcome `hackerman
tree`:

[hackerman tree](https://github.com/pacak/hackerman/blob/master/doc/tree.md)

### Hackerman dupes

`cargo hackerman dupes` lists all the packages used in workspace dependencies present in more
than one version

[hackerman dupes](https://github.com/pacak/hackerman/blob/master/doc/dupes.md)

### Hackerman check

`cargo hackerman check` checks for any issues with unification and reports if fixes are
required

[hackerman check](https://github.com/pacak/hackerman/blob/master/doc/check.md)


### Hackerman show

[hackerman show](https://github.com/pacak/hackerman/blob/master/doc/show.md)



- explaining why a certain crate is included
- explaining why a certain feature of a crate is included (but a bit derpy)
- feature pre-unification



check.md
dupes.md
explain.md
show.md
tree.md

