Tree starts at a given crate/feature and follows dependencies links all the way to the end.

Red nodes represent workspace members, green nodes represent starting features.
Dotted line represents dev-only dependency, dashed line - both dev and normal but
with different features across them.

If a crate is present in several versions you need to specify the
version of one you are interested in, otherwise it's optional.

You can also specify which feature to look for, otherwise hackerman
will be looking for all of them.

Examples:

    cargo hackerman tree rand 0.8.4
    cargo hackerman explain serde_json preserve_order

See also "hackerman tree"
