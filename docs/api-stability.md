# API stability

BoxFerry is pre-1.0. The supported facade and component crates publish one lockstep version.

- Patch releases preserve documented source compatibility.
- Minor releases may remove or replace public APIs and include migration notes.
- Deprecated compatibility layers are not retained without a demonstrated need.
- Public enums are non-exhaustive where future variants are expected.
- Raising Rust 1.85.0 requires a minor release, release notes, and CI evidence.

The complete contract is in [Library API and stability](library-api.md). Native catalogue updates
are evidence changes and do not automatically imply a Rust API break.
