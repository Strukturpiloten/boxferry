//! Shared support for repository-level integration tests.

mod actions;
mod corpus;
mod fixtures;

pub(crate) use actions::{validate_action_pins, validate_repository_supply_chain};
pub(crate) use corpus::validate_real_world_compose_catalog;
pub(crate) use fixtures::{validate_fixture_manifest_text, validate_fixture_tree};
