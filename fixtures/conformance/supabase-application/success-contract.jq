def contains($values; $candidate):
  any($values[]; . == $candidate);

def tuple($code; $subject; $decision):
  {
    code: $code,
    severity: "warning",
    subject: $subject,
    decision: $decision,
  };

def rejection_route:
  ($input == "compose" or $input == "quadlet") and $output == "podman";

def rejection_reason:
  "an image source is not a strict supported Podman image reference";

def rejection_tuple($subject):
  {
    code: "BFP0008",
    severity: "error",
    subject: $subject,
    decision: "rejected",
    reason: rejection_reason,
  };

def selected_services:
  if $selection == "exact" then
    ["auth", "db", "functions", "imgproxy", "kong", "meta", "realtime", "rest", "storage", "studio"]
  elif $selection == "storage" then
    ["db", "imgproxy", "rest", "storage"]
  elif $selection == "label" then
    ["auth", "db", "functions", "imgproxy", "kong", "meta", "realtime", "rest", "storage", "studio", "supavisor"]
  elif $selection == "all" then
    ["auth", "boundary-peer", "db", "functions", "imgproxy", "kong", "meta", "realtime", "rest", "storage", "studio", "supavisor"]
  else
    null
  end;

def service_images:
  {
    auth: "docker.io/supabase/gotrue:v2.189.0@sha256:385184459f57569c54c25209f51f3b2be99ddd7c4ce9e3555b5d3eea8447b7cf",
    db: "docker.io/supabase/postgres:17.6.1.136@sha256:f371b5f3f2ac0a05703f33d6e6134515fb2498cab708fb948a0aeb7481467c00",
    functions: "docker.io/supabase/edge-runtime:v1.74.0@sha256:2781daf92394db91f7e94129cc3d04ec474ad16a8fe64b3fbeef6e7d557ab120",
    imgproxy: "docker.io/darthsim/imgproxy:v3.30.1@sha256:3b709e4a0e5e8e0e959b556b7031229202b4b8e7e7d955c517ea7abed68ee34d",
    kong: "docker.io/kong/kong:3.9.3@sha256:9a2ae6699a2ce0d60592eb176555d3594a22782c20cc6557a61ff3a7e8b559a3",
    meta: "docker.io/supabase/postgres-meta:v0.96.6@sha256:a84cc713585eea7b401e4a2561ec4a1e48c87083d1c7ecb4502f204bb4391300",
    realtime: "docker.io/supabase/realtime:v2.102.3@sha256:aa1c92c0cf326007563641730ec9da9c60478caa6853887775365fa2c097a471",
    rest: "docker.io/postgrest/postgrest:v14.12@sha256:54000f24847d01a2c2302e0041cf0618b875c57fb48507d743cfa9aaa50bf43c",
    storage: "docker.io/supabase/storage-api:v1.60.4@sha256:c8eb9858eafec891a97c27125470aaad54703c3f4eb4d55ca7f1bf6c6411febf",
    studio: "docker.io/supabase/studio:2026.08.03-sha-022b374@sha256:606aca9fdaa753b60968d5c304e2ada83869b76c9043684e63d5885aca9550e8",
    supavisor: "docker.io/supabase/supavisor:2.9.5@sha256:31c2f05b13b11069660fdfae2f6cfd37b509748d2710aca121cfee8b16cb8b07",
  };

def selected_images:
  [
    selected_services[] as $service |
    service_images[$service] |
    select(. != null)
  ];

def unusable_bind_mounts:
  {
    db: [1],
    functions: [0],
    kong: [0],
    studio: [0],
  };

def selected_volumes:
  if $selection == "storage" then
    ["pgdata", "storage"]
  elif $selection == "exact" or $selection == "label" or $selection == "all" then
    ["deno-cache", "pgdata", "storage"]
  else
    null
  end;

def selected_networks:
  if $selection == "storage" then
    ["backend"]
  elif $selection == "exact" or $selection == "label" or $selection == "all" then
    ["backend", "edge"]
  else
    null
  end;

def rejection_image_diagnostics:
  [
    selected_services[] |
    rejection_tuple("services." + $resource_prefix + . + ".image")
  ];

def healthcheck_services:
  ["auth", "db", "functions", "imgproxy", "kong", "realtime", "rest", "storage", "studio"];

def import_service_fields($service):
  (
    if $service == "boundary-peer" then
      []
    else
      [["environment", "approximated"]]
    end
  ) + [
    ["namespaces", "not-promoted"],
    ["networks", "approximated"],
    ["resource_controls", "not-promoted"],
    ["security", "not-promoted"]
  ] + (
    if $service == "db" then
      [["mounts[0]", "approximated"], ["mounts[1]", "not-promoted"]]
    elif $service == "functions" then
      [["mounts[0]", "not-promoted"], ["mounts[1]", "approximated"]]
    elif $service == "imgproxy" or $service == "storage" then
      [["mounts[0]", "approximated"]]
    elif $service == "kong" or $service == "studio" then
      [["mounts[0]", "not-promoted"]]
    else
      []
    end
  );

def import_diagnostics:
  ([
    selected_services[] |
    tuple("BFP0002"; "container:" + $resource_prefix + .; "omitted")
  ] + [
    selected_volumes[] |
    tuple("BFP0002"; "volume:" + $resource_prefix + .; "omitted")
  ] + [
    selected_images[] |
    tuple("BFP0002"; "image:" + .; "omitted")
  ] + [
    unusable_bind_mounts | to_entries[] |
    .key as $service |
    select(contains(selected_services; $service)) |
    .value[] |
    tuple(
      "BFP0002";
      "services." + $resource_prefix + $service + ".mounts[" + (. | tostring) + "].destination";
      "omitted"
    )
  ] + [
    selected_services[] as $service |
    import_service_fields($service)[] |
    tuple("BFP0003"; "services." + $resource_prefix + $service + "." + .[0]; .[1])
  ] + [
    healthcheck_services[] |
    select(. as $service | contains(selected_services; $service)) |
    tuple("BFP0003"; "services." + $resource_prefix + . + ".healthcheck"; "approximated")
  ] + [
    unusable_bind_mounts | to_entries[] |
    .key as $service |
    select(contains(selected_services; $service)) |
    .value[] |
    tuple(
      "BFP0003";
      "services." + $resource_prefix + $service + ".mounts[" + (tostring) + "].writable";
      "not-promoted"
    )
  ] + [
    selected_networks[] |
    tuple("BFP0003"; "networks." + $resource_prefix + . + ".internal"; "approximated")
  ] + [
    selected_volumes[] as $volume |
    ["anonymous", "driver", "gid", "uid"][] |
    tuple("BFP0003"; "volumes." + $resource_prefix + $volume + "." + .; "not-promoted")
  ]);

def dependencies:
  {
    auth: ["db"],
    functions: ["db"],
    kong: ["auth", "functions", "realtime", "rest", "storage", "studio"],
    meta: ["db"],
    realtime: ["db"],
    rest: ["db"],
    storage: ["db", "imgproxy", "rest"],
    studio: ["meta"],
    supavisor: ["db"],
  };

def compose_diagnostics:
  [
    dependencies | to_entries[] |
    .key as $service |
    .value as $candidates |
    select(contains(selected_services; $service)) |
    [$candidates[] | select(. as $dependency | contains(selected_services; $dependency))] |
    to_entries[] |
    tuple(
      "BFC0007";
      "services." + $resource_prefix + $service + ".dependencies[" + (.key | tostring) + "]";
      null
    )
  ];

def compose_healthcheck_diagnostics:
  [
    healthcheck_services[] |
    select(. as $service | contains(selected_services; $service)) |
    tuple("BFC0007"; "services." + $resource_prefix + . + ".healthcheck"; null)
  ];

def compose_image_diagnostics:
  [
    selected_services[] |
    tuple("BFC0009"; "services." + $resource_prefix + . + ".image"; null)
  ];

def compose_network_diagnostics:
  if $input == "podman" and $selection == "all" then
    [tuple("BFC0007"; "networks." + $resource_prefix + "edge.internal"; null)]
  else
    []
  end;

def compose_output_diagnostics:
  compose_image_diagnostics +
  if $input == "compose" then
    []
  else
    compose_diagnostics + compose_healthcheck_diagnostics + compose_network_diagnostics
  end;

def podman_environment_fields:
  {
    auth: ["GOTRUE_DB_DATABASE_URL", "GOTRUE_DB_DRIVER"],
    db: ["POSTGRES_DB", "POSTGRES_PASSWORD", "POSTGRES_USER"],
    functions: ["SUPABASE_DB_URL", "SUPABASE_URL", "VERIFY_JWT"],
    imgproxy: ["IMGPROXY_BIND", "IMGPROXY_LOCAL_FILESYSTEM_ROOT"],
    meta: ["PG_META_DB_HOST", "PG_META_DB_NAME"],
    realtime: ["DB_HOST", "DB_NAME", "DB_PASSWORD"],
    rest: ["PGRST_DB_SCHEMAS", "PGRST_DB_URI"],
    storage: ["DATABASE_URL", "IMGPROXY_URL", "POSTGREST_URL", "STORAGE_BACKEND"],
    studio: ["STUDIO_PG_META_URL", "SUPABASE_URL"],
    supavisor: ["POSTGRES_DB", "POSTGRES_HOST"],
  };

def generated_environment_fields:
  podman_environment_fields +
  {kong: ["KONG_DATABASE", "KONG_DECLARATIVE_CONFIG"]};

def rejection_network_subjects:
  if $selection == "storage" or ($input == "quadlet" and $selection == "all") then
    ["networks." + $resource_prefix + "backend.internal"]
  elif $input == "compose" and $selection == "all" then
    [
      "networks." + $resource_prefix + "backend.internal",
      "networks." + $resource_prefix + "edge.runtime_name"
    ]
  else
    [
      "networks." + $resource_prefix + "backend.internal",
      "networks." + $resource_prefix + "edge.internal"
    ]
  end;

def generated_podman_diagnostics($include_healthchecks):
  ([
    rejection_network_subjects[] |
    tuple("BFP0007"; .; "omitted")
  ] + [
    generated_environment_fields | to_entries[] |
    .key as $service |
    select(contains(selected_services; $service)) |
    .value[] |
    tuple(
      "BFP0007";
      "services." + $resource_prefix + $service + ".environment." + .;
      "omitted"
    )
  ] + (
    if $include_healthchecks then
      [
        healthcheck_services[] |
        select(. as $service | contains(selected_services; $service)) |
        tuple("BFP0007"; "services." + $resource_prefix + . + ".healthcheck"; "omitted")
      ]
    else
      []
    end
  ) + (
    if contains(selected_services; "kong") then
      [tuple("BFP0007"; "services." + $resource_prefix + "kong.ports"; "omitted")]
    else
      []
    end
  ));

def quadlet_dependency_diagnostics:
  [
    dependencies | to_entries[] |
    .key as $service |
    .value as $candidates |
    select(contains(selected_services; $service)) |
    [$candidates[] | select(. as $dependency | contains(selected_services; $dependency))] |
    to_entries[] |
    tuple(
      "BFP0007";
      "services." + $resource_prefix + $service + ".dependencies[" + (.key | tostring) + "].options";
      "omitted"
    )
  ];

def rejection_source_diagnostics:
  if $input == "compose" then
    generated_podman_diagnostics(false)
  elif $input == "quadlet" then
    generated_podman_diagnostics(true) + quadlet_dependency_diagnostics
  else
    null
  end;

def rejection_diagnostics:
  rejection_image_diagnostics + rejection_source_diagnostics;

def podman_diagnostics:
  ([
    selected_networks[] |
    tuple("BFP0007"; "networks." + $resource_prefix + . + ".internal"; "omitted")
  ] + [
    generated_environment_fields | to_entries[] |
    .key as $service |
    select(contains(selected_services; $service)) |
    .value[] |
    tuple("BFP0007"; "services." + $resource_prefix + $service + ".environment." + .; "omitted")
  ] + [
    healthcheck_services[] |
    select(. as $service | contains(selected_services; $service)) |
    tuple("BFP0007"; "services." + $resource_prefix + . + ".healthcheck"; "omitted")
  ] + (
    if contains(selected_services; "kong") then
      [tuple("BFP0007"; "services." + $resource_prefix + "kong.ports"; "omitted")]
    else
      []
    end
  ));

def expected_diagnostics:
  if selected_services == null or selected_volumes == null or selected_networks == null then
    null
  elif rejection_route then
    rejection_diagnostics
  elif $input == "podman" and $output == "compose" then
    import_diagnostics + compose_output_diagnostics
  elif $input == "podman" and $output == "quadlet" then
    import_diagnostics
  elif $input == "podman" and $output == "podman" then
    import_diagnostics + podman_diagnostics
  elif $input == "quadlet" and $output == "compose" then
    compose_output_diagnostics
  elif $input == "compose" and $output == "compose" then
    compose_output_diagnostics
  elif ($input == "compose" and $output == "quadlet") or
      ($input == "quadlet" and $output == "quadlet") then
    []
  else
    null
  end;

def field($diagnostic; $name):
  [$diagnostic.fields[]? | select(.name == $name) | .value] as $values |
  if ($values | length) == 1 then $values[0] else null end;

def actual_diagnostics:
  [
    .diagnostics[]? |
    . as $diagnostic |
    {
      code: .code,
      severity: .severity,
      subject: field($diagnostic; "subject"),
      decision: field($diagnostic; "decision"),
    }
  ];

def actual_rejection_diagnostics:
  [
    .diagnostics[]? |
    . as $diagnostic |
    (
      {
        code: .code,
        severity: .severity,
        subject: field($diagnostic; "subject"),
        decision: field($diagnostic; "decision"),
      } +
      if .code == "BFP0008" then
        {reason: field($diagnostic; "reason")}
      else
        {}
      end
    )
  ];

def import_service_outcomes($service):
  (
    if $service == "boundary-peer" then
      []
    else
      ["approximate"]
    end
  ) + [
    "unsupported",
    "approximate",
    "unsupported",
    "unsupported"
  ] + (
    if $service == "db" then
      ["approximate", "unsupported"]
    elif $service == "functions" then
      ["unsupported", "approximate"]
    elif $service == "imgproxy" or $service == "storage" then
      ["approximate"]
    elif $service == "kong" or $service == "studio" then
      ["unsupported"]
    else
      []
    end
  );

def import_outcomes:
  ([selected_services[] | "unsupported"] +
  [selected_volumes[] | "unsupported"] +

  [selected_images[] | "unsupported"] +

  [
    unusable_bind_mounts | to_entries[] |
    select(.key as $service | contains(selected_services; $service)) |
    .value[] |
    ["unsupported", "unsupported"][]
  ] +
  [
    selected_services[] as $service |
    import_service_outcomes($service)[]
  ] +
  [
    healthcheck_services[] |
    select(. as $service | contains(selected_services; $service)) |
    "approximate"
  ] +

  [selected_networks[] | "approximate"] +
  [
    selected_volumes[] |
    ["anonymous", "driver", "gid", "uid"][] |
    "unsupported"
  ] +
  [selected_volumes[] | "unsupported"] +
  (
    if contains(selected_services; "kong") then
      ["approximate", "unsupported"]
    else
      []
    end
  ));

def compose_outcomes:
  [
    dependencies | to_entries[] |
    .key as $service |
    .value as $candidates |
    select(contains(selected_services; $service)) |
    $candidates[] |
    select(. as $dependency | contains(selected_services; $dependency)) |
    "unsupported"
  ];

def compose_output_outcomes:
  ([selected_services[] | "approximate"] +
  if $input == "compose" then
    []
  else
    compose_outcomes +
    [
      healthcheck_services[] |
      select(. as $service | contains(selected_services; $service)) |
      "unsupported"
    ] +
    if $input == "podman" and $selection == "all" then
      ["unsupported"]
    else
      []
    end
  end);

def podman_outcomes:
  ([selected_networks[] | "unsupported"] +
  [
    generated_environment_fields | to_entries[] |
    .key as $service |
    select(contains(selected_services; $service)) |
    .value[] |
    "unsupported"
  ] +
  [
    healthcheck_services[] |
    select(. as $service | contains(selected_services; $service)) |
    "unsupported"
  ] +
  (
    if contains(selected_services; "kong") then
      ["unsupported"]
    else
      []
    end
  ));

def expected_success_outcomes:
  if selected_services == null or selected_volumes == null or selected_networks == null then
    null
  elif $input == "podman" and $output == "compose" then
    import_outcomes + compose_output_outcomes
  elif $input == "podman" and $output == "quadlet" then
    import_outcomes
  elif $input == "podman" and $output == "podman" then
    import_outcomes + podman_outcomes
  elif $input == "quadlet" and $output == "compose" then
    compose_output_outcomes
  elif $input == "compose" and $output == "compose" then
    compose_output_outcomes
  elif ($input == "compose" and $output == "quadlet") or
       ($input == "quadlet" and $output == "quadlet") then
    []
  else
    null
  end;

def expected_success_fidelity:
  expected_success_outcomes as $outcomes |
  if $outcomes == null then
    null
  else
    {
      approximate: ([$outcomes[] | select(. == "approximate")] | length),
      unsupported: ([$outcomes[] | select(. == "unsupported")] | length),
      invalid: 0,
      other: 0,
    }
  end;

def expected_rejection_fidelity:
  rejection_diagnostics as $diagnostics |
  if $diagnostics == null then
    null
  else
    {
      approximate: 0,
      unsupported: ([$diagnostics[] | select(.code == "BFP0007")] | length),
      invalid: ([$diagnostics[] | select(.code == "BFP0008")] | length),
      other: 0,
    }
  end;

def exact_fidelity_shape:
  (.fidelity | type) == "object" and
  (.fidelity | keys) == ["approximate", "exact", "invalid", "other", "unsupported"] and
  (.fidelity.exact | type == "number" and . >= 0 and floor == .) and
  (.fidelity.approximate | type == "number" and . >= 0 and floor == .) and
  (.fidelity.unsupported | type == "number" and . >= 0 and floor == .) and
  (.fidelity.invalid | type == "number" and . >= 0 and floor == .) and
  (.fidelity.other | type == "number" and . >= 0 and floor == .);

def success_contract($expected):
  actual_diagnostics as $actual |
  expected_success_fidelity as $fidelity |
  .schema_version == 1 and
  .status == "success" and
  ($actual | sort_by(.code, .severity, .subject, .decision)) ==
    ($expected | sort_by(.code, .severity, .subject, .decision)) and
  exact_fidelity_shape and
  .fidelity.approximate == $fidelity.approximate and
  .fidelity.unsupported == $fidelity.unsupported and
  .fidelity.invalid == $fidelity.invalid and
  .fidelity.other == $fidelity.other and
  all(.diagnostics[]?; (.name | length) > 0);

def rejection_contract($expected):
  actual_rejection_diagnostics as $actual |
  expected_rejection_fidelity as $fidelity |
  ($expected | map(select(.code == "BFP0008") | .subject)) as $expected_subjects |
  ($actual | map(select(.code == "BFP0008") | .subject)) as $actual_subjects |
  .schema_version == 1 and
  .status == "failure" and
  .exit_category == "input-or-execution" and
  .primary_diagnostic_code == "BFP0008" and
  (.output_artifacts | type) == "array" and
  (.output_artifacts | length) == 0 and
  ($expected_subjects | length) > 0 and
  ($expected_subjects | length) == ($expected_subjects | unique | length) and
  ($actual_subjects | length) == ($actual_subjects | unique | length) and
  ($actual | sort_by(.code, .severity, .subject, .decision, .reason)) ==
    ($expected | sort_by(.code, .severity, .subject, .decision, .reason)) and
  exact_fidelity_shape and
  .fidelity.approximate == $fidelity.approximate and
  .fidelity.unsupported == $fidelity.unsupported and
  .fidelity.invalid == $fidelity.invalid and
  .fidelity.other == $fidelity.other and
  all(.diagnostics[]?; (.name | length) > 0);

if $emit_expected == "fidelity" then
  if rejection_route then expected_rejection_fidelity else expected_success_fidelity end
elif $emit_expected == true then
  expected_diagnostics
else
  expected_diagnostics as $expected |
  $expected != null and
  if rejection_route then rejection_contract($expected) else success_contract($expected) end
end
