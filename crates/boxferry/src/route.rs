//! Finite, typed CLI conversion routes derived from independent format axes.

macro_rules! format_axis {
    (
        $(#[$enum_meta:meta])*
        pub(crate) enum $name:ident {
            $(
                $(#[$variant_meta:meta])*
                $variant:ident => $label:literal
            ),+ $(,)?
        }
    ) => {
        $(#[$enum_meta])*
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
        pub(crate) enum $name {
            $(
                $(#[$variant_meta])*
                $variant,
            )+
        }

        impl $name {
            pub(crate) const ALL: &'static [Self] = &[
                $(Self::$variant,)+
            ];

            pub(crate) const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $label,)+
                }
            }
        }
    };
}

format_axis! {
    /// Native input formats accepted by the document conversion CLI.
    pub(crate) enum InputType {
        Compose => "compose",
        Podman => "podman",
        Quadlet => "quadlet",
    }
}

format_axis! {
    /// Native output formats accepted by the document conversion CLI.
    pub(crate) enum OutputType {
        Compose => "compose",
        Podman => "podman",
        Quadlet => "quadlet",
    }
}

/// Target-version selection family required by one output format.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TargetSelector {
    PodmanRange,
    PodmanMaximum,
    ComposeSpecification,
}

impl OutputType {
    pub(crate) const fn target_selector(self) -> TargetSelector {
        match self {
            Self::Compose => TargetSelector::ComposeSpecification,
            Self::Quadlet => TargetSelector::PodmanRange,
            Self::Podman => TargetSelector::PodmanMaximum,
        }
    }
}

/// One route in the complete input-by-output product.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RouteSpec {
    pub(crate) input: InputType,
    pub(crate) output: OutputType,
    pub(crate) target_selector: TargetSelector,
    pub(crate) exact_boundary: &'static str,
    pub(crate) approximate_boundaries: &'static [&'static str],
    pub(crate) policy_controlled_boundaries: &'static [&'static str],
}

impl RouteSpec {
    const fn new(input: InputType, output: OutputType) -> Self {
        let (exact_boundary, approximate_boundaries, policy_controlled_boundaries) = fidelity_boundaries(input, output);
        Self {
            input,
            output,
            target_selector: output.target_selector(),
            exact_boundary,
            approximate_boundaries,
            policy_controlled_boundaries,
        }
    }

    pub(crate) const fn source_name(self) -> &'static str {
        self.input.name()
    }

    pub(crate) const fn target_name(self) -> &'static str {
        self.output.name()
    }
}

const fn fidelity_boundaries(
    input: InputType,
    output: OutputType,
) -> (&'static str, &'static [&'static str], &'static [&'static str]) {
    match (input, output) {
        (InputType::Compose, OutputType::Compose) => (
            "supported-compose-compose-specification-intersection",
            &[],
            &["unsupported-fields"],
        ),
        (InputType::Compose, OutputType::Quadlet) => (
            "supported-compose-quadlet-intersection",
            &["pod-grouping"],
            &["unsupported-fields"],
        ),
        (InputType::Compose, OutputType::Podman) => (
            "supported-compose-podman-intersection",
            &["pod-grouping"],
            &["unsupported-fields"],
        ),
        (InputType::Quadlet, OutputType::Compose) => (
            "supported-quadlet-compose-specification-intersection",
            &["environment-file-reconstruction"],
            &["unsupported-fields"],
        ),
        (InputType::Quadlet, OutputType::Quadlet) => (
            "supported-quadlet-canonical-subset",
            &["environment-file-reconstruction", "systemd-runtime-semantics"],
            &["unsupported-fields"],
        ),
        (InputType::Quadlet, OutputType::Podman) => (
            "supported-quadlet-podman-intersection",
            &["environment-file-reconstruction", "systemd-runtime-semantics"],
            &["unsupported-fields"],
        ),
        (InputType::Podman, OutputType::Compose) => (
            "supported-podman-compose-specification-intersection",
            &["effective-state-promotion"],
            &["unmodelled-fields", "incomplete-secret-grants"],
        ),
        (InputType::Podman, OutputType::Quadlet) => (
            "supported-podman-quadlet-intersection",
            &["effective-state-promotion"],
            &["unmodelled-fields", "incomplete-secret-grants"],
        ),
        (InputType::Podman, OutputType::Podman) => (
            "supported-podman-observation-deployment-intersection",
            &["effective-state-promotion"],
            &["unmodelled-fields", "incomplete-secret-grants"],
        ),
    }
}

/// Returns every CLI route as the Cartesian product of registered inputs and outputs.
pub(crate) fn routes() -> impl Iterator<Item = RouteSpec> {
    InputType::ALL.iter().copied().flat_map(|input| {
        OutputType::ALL
            .iter()
            .copied()
            .map(move |output| RouteSpec::new(input, output))
    })
}

/// Looks up the route selected by typed CLI axes.
pub(crate) const fn find(input: InputType, output: OutputType) -> RouteSpec {
    RouteSpec::new(input, output)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{InputType, OutputType, find, routes};

    #[test]
    fn registry_is_the_complete_unique_input_output_product() {
        let registered = routes()
            .map(|route| (route.input, route.output))
            .collect::<BTreeSet<_>>();
        let expected = InputType::ALL
            .iter()
            .copied()
            .flat_map(|input| OutputType::ALL.iter().copied().map(move |output| (input, output)))
            .collect::<BTreeSet<_>>();

        assert_eq!(registered, expected);
        assert_eq!(registered.len(), InputType::ALL.len() * OutputType::ALL.len());
        for (input, output) in expected {
            assert_eq!((find(input, output).input, find(input, output).output), (input, output));
        }
    }
}
