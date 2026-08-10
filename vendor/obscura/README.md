# Obscura build patches

Obscura v0.2.0 carries unreleased patches to `taffy` and `cosmic-text`. Cargo ignores a dependency's `[patch.crates-io]` section, so Scorch vendors those two patched crates to make the pinned Obscura library build reproducibly.

The sources are copied from Obscura commit `97124edeb2ea610615e78f43e097454e3b221f6b`. Their original license files are preserved in each directory.
