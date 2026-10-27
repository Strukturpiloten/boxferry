//! CLI transport integration; native parsing and semantic conversion remain separate.
use super::route::{self, InputType, OutputType};
use super::{
    CliLossPolicy, ConversionPolicyOptions, EnvironmentValuePolicy, ExtractQuadlet, GenericConversion, Grouping,
    InputDocuments, InputRouteOptions, OrderedInput, OutputLayout, OutputRouteOptions, PodmanSelector,
    QuadletInputOptions, QuadletOutputOptions, RenderedConversion, RenderedFile, ReportAliases, ResolvedInput,
    StructuredFailure, discover_quadlet_directory, finish_invocation, is_supported_quadlet_path, new_report,
    output_write_diagnostic, resolve_regular_file, resolved_report_context, sanitized_diagnostic,
};
use boxferry::{
    RuleId,
    report::{ExitCategory, FailedStage, InvocationReport, OutputArtifact, ReportChoice, ReportStatus, VersionBounds},
};
use boxferry::{
    quadlet_bundle::{BundleError, MAX_BUNDLE_BYTES, QuadletBundle, RESERVED_MAGIC},
    report::{BundleMember, InputBundleMember},
};
use std::{
    collections::BTreeSet,
    error::Error,
    fs,
    io::{self, Read},
    path::Path,
    process::ExitCode,
    sync::Arc,
};

pub(super) fn render_quadlet(
    result: boxferry::ConversionResult<boxferry::QuadletOutput>,
    layout: OutputLayout,
    protected_config: bool,
) -> RenderedConversion {
    let (output, outcomes, diagnostics) = result.into_parts();
    let mut conversion = RenderedConversion {
        output: None,
        bundle_members: Vec::new(),
        private_output: protected_config || matches!(layout, OutputLayout::QuadletBundle),
        transport_error: None,
        outcomes,
        diagnostics,
    };
    if let Some(output) = output {
        match layout {
            OutputLayout::Files => {
                conversion.output = Some(
                    output
                        .files()
                        .iter()
                        .map(|file| RenderedFile {
                            name: file.name().as_str().to_owned(),
                            text: file.text().to_owned(),
                        })
                        .collect(),
                );
            }
            OutputLayout::QuadletBundle => {
                let encoded = QuadletBundle::from_output(&output).and_then(|bundle| {
                    let text = String::from_utf8(bundle.encode()?).map_err(|_| BundleError::InvalidUtf8)?;
                    Ok((text, member_reports(&bundle, "quadlet.bundle")))
                });
                match encoded {
                    Ok((text, members)) => {
                        conversion.bundle_members = members;
                        conversion.output = Some(vec![RenderedFile {
                            name: "quadlet.bundle".into(),
                            text,
                        }]);
                    }
                    Err(error) => conversion.transport_error = Some(error),
                }
            }
        }
    }
    conversion
}

fn member_reports(bundle: &QuadletBundle, transport: &str) -> Vec<BundleMember> {
    bundle
        .members()
        .iter()
        .map(|member| BundleMember {
            transport: transport.into(),
            name: member.name().into(),
            unit_type: unit_type_name(member.unit_type()).into(),
            size: u64::try_from(member.text().len()).unwrap_or(u64::MAX),
        })
        .collect()
}
fn unit_type_name(kind: boxferry::quadlet::quadlet_lens::model::QuadletUnitType) -> &'static str {
    use boxferry::quadlet::quadlet_lens::model::QuadletUnitType;
    match kind {
        QuadletUnitType::Container => "container",
        QuadletUnitType::Network => "network",
        QuadletUnitType::Volume => "volume",
        QuadletUnitType::Pod => "pod",
        QuadletUnitType::Image => "image",
        QuadletUnitType::Build => "build",
        QuadletUnitType::Kube => "kube",
        QuadletUnitType::Artifact => "artifact",
        _ => "unknown",
    }
}
pub(super) fn input_members(inputs: &[ResolvedInput]) -> Vec<InputBundleMember> {
    inputs
        .iter()
        .enumerate()
        .filter_map(|(index, input)| match input {
            ResolvedInput::BundleMember { transport_index, .. } => Some(InputBundleMember {
                transport_alias: transport_alias(*transport_index),
                member_alias: format!("<input-{}>", index + 1),
            }),
            _ => None,
        })
        .collect()
}
pub(super) fn transport_alias(index: usize) -> String {
    format!("<bundle-input-{}>", index + 1)
}
fn read_bounded(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(u64::try_from(MAX_BUNDLE_BYTES + 1).map_err(io::Error::other)?)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}
fn read_bundle(path: &Path) -> io::Result<Option<QuadletBundle>> {
    let mut file =
        fs::File::open(path).map_err(|error| io::Error::new(error.kind(), "Quadlet input could not be read"))?;
    let mut prefix = Vec::new();
    Read::by_ref(&mut file)
        .take(u64::try_from(RESERVED_MAGIC.len()).map_err(io::Error::other)?)
        .read_to_end(&mut prefix)?;
    if prefix != RESERVED_MAGIC {
        return Ok(None);
    }
    let remaining = MAX_BUNDLE_BYTES + 1 - prefix.len();
    file.take(u64::try_from(remaining).map_err(io::Error::other)?)
        .read_to_end(&mut prefix)?;
    QuadletBundle::decode(&prefix)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
pub(super) fn resolve_inputs(ordered: Vec<OrderedInput>) -> io::Result<Vec<ResolvedInput>> {
    let mut physical = Vec::new();
    for item in ordered {
        match item {
            OrderedInput::File(path) if path == Path::new("-") => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "stdin is not supported for Quadlet input",
                ));
            }
            OrderedInput::File(path) => physical.push(ResolvedInput::File(resolve_regular_file(&path)?)),
            OrderedInput::Directory(path) => physical.extend(discover_quadlet_directory(&path)?),
            OrderedInput::ImplicitCompose => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "implicit Compose input is not Quadlet input",
                ));
            }
        }
    }
    let mut paths = BTreeSet::new();
    let mut expanded = Vec::new();
    let mut has_bundle = false;
    for (transport_index, input) in physical.into_iter().enumerate() {
        let path = input
            .path()
            .ok_or_else(|| io::Error::other("Quadlet input has no path"))?;
        if !paths.insert(fs::canonicalize(path)?) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "duplicate resolved Quadlet input",
            ));
        }
        if matches!(input, ResolvedInput::File(_))
            && let Some(bundle) = read_bundle(path)?
        {
            has_bundle = true;
            expanded.extend(
                bundle
                    .members()
                    .iter()
                    .cloned()
                    .map(|member| ResolvedInput::BundleMember {
                        transport: path.to_owned(),
                        transport_index,
                        member: Arc::new(member),
                    }),
            );
        } else {
            if !is_supported_quadlet_path(path) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Quadlet input must use a supported lower-case unit extension or explicit BoxFerry bundle",
                ));
            }
            expanded.push(input);
        }
    }
    let mut names = BTreeSet::new();
    for input in &expanded {
        let name = input
            .native_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Quadlet input filename is not UTF-8"))?;
        let identity = if has_bundle {
            name.to_ascii_lowercase()
        } else {
            name.to_owned()
        };
        if !names.insert(identity) {
            return Err(if has_bundle {
                io::Error::new(io::ErrorKind::InvalidData, BundleError::MemberCollision)
            } else {
                io::Error::new(io::ErrorKind::InvalidInput, "duplicate Quadlet unit basename")
            });
        }
    }
    Ok(expanded)
}
pub(super) fn input_failure(
    error: io::Error,
    arguments: &GenericConversion,
    aliases: &ReportAliases,
) -> Box<dyn Error> {
    if let Some(bundle) = error.get_ref().and_then(|error| error.downcast_ref::<BundleError>()) {
        let inputs = arguments
            .input_files
            .iter()
            .cloned()
            .map(ResolvedInput::File)
            .collect::<Vec<_>>();
        return Box::new(StructuredFailure::with_context(
            FailedStage::InputDiscovery,
            vec![sanitized_diagnostic(
                RuleId::OrchestrationFailed,
                "error",
                "Quadlet transport input is invalid",
                &[
                    ("transport", "quadlet-bundle", false),
                    ("reason", &bundle.to_string(), false),
                ],
                aliases,
            )],
            &inputs,
        ));
    }
    Box::new(error)
}

pub(super) fn run_extract(command: ExtractQuadlet, matches: &clap::ArgMatches) -> Result<ExitCode, Box<dyn Error>> {
    let directory = command.destination.output_directory;
    let input = command.input_files;
    let arguments = GenericConversion::from_route(
        InputType::Quadlet,
        OutputType::Quadlet,
        InputDocuments {
            input_files: vec![input.clone()],
            input_directories: Vec::new(),
        },
        InputRouteOptions::Quadlet(QuadletInputOptions { application_name: None }),
        OutputRouteOptions::Quadlet(QuadletOutputOptions {
            podman_minimum_version: PodmanSelector::minimum_default(),
            podman_maximum_version: PodmanSelector::maximum_default(),
            grouping: Grouping::Separate,
            pod_name: None,
            output_layout: OutputLayout::Files,
        }),
        ConversionPolicyOptions {
            loss_policy: CliLossPolicy::Exact,
            environment_values: EnvironmentValuePolicy::Withhold,
        },
        command.diagnostics,
    );
    let aliases = ReportAliases::for_invocation(&arguments, Some(&directory));
    let mut report = new_report(&arguments, route::find(InputType::Quadlet, OutputType::Quadlet));
    report.operation = "extract".into();
    report.requested_versions = VersionBounds {
        minimum: "not-applicable".into(),
        maximum: "not-applicable".into(),
    };
    report.resolved_versions = report.requested_versions.clone();
    report.choices = vec![ReportChoice {
        name: "output_layout".into(),
        value: "files".into(),
    }];
    let inputs = vec![ResolvedInput::File(input.clone())];
    (report.inputs, report.discovery) = resolved_report_context(&inputs);
    for input in &mut report.inputs {
        input.kind = "quadlet-bundle".into();
    }
    let code = match extract(&input, &directory, &mut report) {
        Ok(()) => {
            report.status = ReportStatus::Success;
            report.exit_category = ExitCategory::Success;
            ExitCode::SUCCESS
        }
        Err(ExtractFailure::Read(error)) => {
            report.failed_stage = Some(FailedStage::InputRead);
            report.diagnostics.push(sanitized_diagnostic(
                RuleId::InputReadFailed,
                "error",
                "Quadlet bundle input could not be read as a regular file",
                &[("reason", &error.kind().to_string(), false)],
                &aliases,
            ));
            ExitCode::FAILURE
        }
        Err(ExtractFailure::Transport(error)) => {
            report.failed_stage = Some(FailedStage::InputDiscovery);
            report.diagnostics.push(sanitized_diagnostic(
                RuleId::OrchestrationFailed,
                "error",
                "Quadlet transport input is invalid",
                &[
                    ("transport", "quadlet-bundle", false),
                    ("reason", &error.to_string(), false),
                ],
                &aliases,
            ));
            ExitCode::FAILURE
        }
        Err(ExtractFailure::Write(error)) => {
            report.failed_stage = Some(FailedStage::OutputWrite);
            report.exit_category = ExitCategory::OutputWrite;
            report.diagnostics.push(output_write_diagnostic(&error, &aliases));
            report.events.push("output-write-failed".into());
            ExitCode::FAILURE
        }
    };
    finish_invocation(
        &arguments,
        matches,
        &aliases,
        (report, code, inputs, None),
        false,
        "extract",
    )
}
enum ExtractFailure {
    Read(io::Error),
    Transport(BundleError),
    Write(io::Error),
}
fn extract(input: &Path, directory: &Path, report: &mut InvocationReport) -> Result<(), ExtractFailure> {
    let input = resolve_regular_file(input).map_err(ExtractFailure::Read)?;
    let bytes = read_bounded(&input).map_err(ExtractFailure::Read)?;
    let bundle = QuadletBundle::decode(&bytes).map_err(ExtractFailure::Transport)?;
    report.bundle_members = member_reports(&bundle, "<input-1>");
    report.output_artifacts = bundle
        .members()
        .iter()
        .map(|member| OutputArtifact {
            name: member.name().into(),
            size: u64::try_from(member.text().len()).unwrap_or(u64::MAX),
        })
        .collect();
    bundle.extract_to(directory).map_err(ExtractFailure::Write)
}
