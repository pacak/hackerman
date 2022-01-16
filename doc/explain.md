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

See also "hackerman tree"
