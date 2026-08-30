# Expand #[corot] on the binary crate root
expand:
    cargo expand

# Expand the serde example (requires --features serde)
expand-serde:
    cargo expand --example with_serde --features serde
