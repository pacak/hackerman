When used with --lock option will take a checksum of all the dependencies and will
save it inside Cargo.toml file under ["package.metadata.hackerman.lock"] and subsequent
calls to check will confirm that this checksum is still valid.

This is required to make sure that original (unhacked) dependencies are saved and can be
restored at a later point.
