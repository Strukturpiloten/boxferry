//! Finite, typed CLI conversion routes.

use clap::ValueEnum;

/// Native input formats accepted by the generic CLI.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum InputType {
    Compose,
    Quadlet,
}

/// Native output formats accepted by the generic CLI.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum OutputType {
    Quadlet,
    Compose,
}

impl InputType {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Compose => "compose",
            Self::Quadlet => "quadlet",
        }
    }
}

impl OutputType {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Quadlet => "quadlet",
            Self::Compose => "compose",
        }
    }
}

/// Target-version selection family required by a route.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TargetSelector {
    PodmanRange,
    ComposeSpecification,
}

/// Typed executor selected by a route entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RouteExecutor {
    ComposeToQuadlet,
    QuadletToCompose,
}

/// Source-side CLI option family accepted by a route.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InputOptions {
    Compose,
    Quadlet,
}

/// One explicitly implemented CLI route.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RouteSpec {
    pub(crate) input: InputType,
    pub(crate) output: OutputType,
    pub(crate) executor: RouteExecutor,
    pub(crate) input_options: InputOptions,
    pub(crate) target_selector: TargetSelector,
    pub(crate) exact_boundary: &'static str,
    pub(crate) approximate_boundaries: &'static [&'static str],
    pub(crate) policy_controlled_boundaries: &'static [&'static str],
}

impl RouteSpec {
    pub(crate) const fn source_name(self) -> &'static str {
        self.input.name()
    }

    pub(crate) const fn target_name(self) -> &'static str {
        self.output.name()
    }
}

const COMPOSE_TO_QUADLET: RouteSpec = RouteSpec {
    input: InputType::Compose,
    output: OutputType::Quadlet,
    executor: RouteExecutor::ComposeToQuadlet,
    input_options: InputOptions::Compose,
    target_selector: TargetSelector::PodmanRange,
    exact_boundary: "supported-compose-quadlet-intersection",
    approximate_boundaries: &["pod-grouping"],
    policy_controlled_boundaries: &["unsupported-fields"],
};

const QUADLET_TO_COMPOSE: RouteSpec = RouteSpec {
    input: InputType::Quadlet,
    output: OutputType::Compose,
    executor: RouteExecutor::QuadletToCompose,
    input_options: InputOptions::Quadlet,
    target_selector: TargetSelector::ComposeSpecification,
    exact_boundary: "supported-quadlet-compose-specification-intersection",
    approximate_boundaries: &["environment-file-reconstruction"],
    policy_controlled_boundaries: &["unsupported-fields"],
};

const ROUTES: &[RouteSpec] = &[COMPOSE_TO_QUADLET, QUADLET_TO_COMPOSE];

/// Returns every CLI route implemented by this build.
pub(crate) const fn routes() -> &'static [RouteSpec] {
    ROUTES
}

/// Looks up an explicitly implemented route; all other pairs are unavailable.
pub(crate) fn find(input: InputType, output: OutputType) -> Option<RouteSpec> {
    ROUTES
        .iter()
        .copied()
        .find(|route| route.input == input && route.output == output)
}
