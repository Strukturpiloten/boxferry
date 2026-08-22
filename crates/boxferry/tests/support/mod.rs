//! Shared support for repository-level integration tests.

mod actions;
mod corpus;
mod fixtures;
#[cfg(all(feature = "cli", feature = "podman"))]
#[allow(dead_code)]
mod podman_cassette;

pub(crate) use actions::{validate_action_pins, validate_repository_supply_chain};
pub(crate) use corpus::validate_real_world_compose_catalog;
pub(crate) use fixtures::{validate_fixture_manifest_text, validate_fixture_tree};
#[cfg(all(feature = "cli", feature = "podman"))]
#[allow(unused_imports)]
pub(crate) use podman_cassette::{PodmanCassette, PodmanCassetteServer};
