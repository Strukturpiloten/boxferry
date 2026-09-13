def contains($values; $candidate):
  any($values[]; . == $candidate);

def tuple($code; $subject; $decision):
  {
    code: $code,
    severity: "warning",
    subject: $subject,
    decision: $decision,
  };

# All nine route families are successful; diagnostics below describe reviewed lossy projections.

def selected_services:
  if $selection == "exact" then
    ["auth", "db", "functions", "imgproxy", "kong", "meta", "realtime", "rest", "storage", "studio", "supavisor"]
  elif $selection == "storage" then
    ["auth", "db", "functions", "imgproxy", "kong", "meta", "realtime", "rest", "storage", "studio", "supavisor"]
  elif $selection == "label" then
    ["auth", "db", "functions", "imgproxy", "kong", "meta", "realtime", "rest", "storage", "studio", "supavisor"]
  elif $selection == "all" then
    ["auth", "boundary-peer", "db", "functions", "imgproxy", "kong", "meta", "realtime", "rest", "storage", "studio", "supavisor"]
  else
    null
  end;

def service_images:
  {
    auth: "docker.io/supabase/gotrue@sha256:0a8557cbe0fd53a067726fe656f79eb1b03a1ab3cdde4b59907ce5a1e1a202ab",
    db: "docker.io/supabase/postgres@sha256:5a4314708484bec672de2c09653a5c01fb1c84a998564ac231b0325e2238ed5b",
    functions: "docker.io/supabase/edge-runtime@sha256:8c17262ecf2fcc43fe19c48d239280592129f2164e61f0f17ba56533120f92d5",
    imgproxy: "docker.io/darthsim/imgproxy@sha256:965c3782818766a477a056016e18f88f9a028bf68b39cb2316978945ac2c0492",
    kong: "docker.io/kong/kong@sha256:61591af560fc9ba4d1e2fcc8be87f28e374c4b2f4a8f0e637702ee12dcaddade",
    meta: "docker.io/supabase/postgres-meta@sha256:b9edad6fff2d4fb991ecd57837dbe3f21d2efa0f0ccb186f6ccf0e2d57192fed",
    realtime: "docker.io/supabase/realtime@sha256:2cc87edf0db5cebf1f58c9a4116bb80a25ff764c8706b6802fa68d976e66e5d7",
    rest: "docker.io/postgrest/postgrest@sha256:63b567a462c4fd81ede0bdff0b38a150f732ad5fe4f4b01cebeb6a1aa8dbe0d6",
    storage: "docker.io/supabase/storage-api@sha256:6f706c1184d97b081446527bb62a3193d3d47ad0daafcf738fd5c3e5a62aed97",
    studio: "docker.io/supabase/studio@sha256:2616bb9ed337963fe27ce682b1783875083537d5fb54bfea4c399fb0c56ff03e",
    supavisor: "docker.io/supabase/supavisor@sha256:4dd940610c0ef5c8284ef88a28530566d45b52f7d3de285497a67c169e00cec9",
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
    functions: [1],
    kong: [0],
    studio: [0],
  };

def observed_mounts:
  {
    db: [0, 1],
    functions: [0, 1],
    imgproxy: [0],
    kong: [0],
    storage: [0],
    studio: [0],
  };

def selected_volumes:
  if $selection == "exact" or $selection == "storage" or $selection == "label" or $selection == "all" then
    ["deno-cache", "pgdata", "storage"]
  else
    null
  end;

def selected_networks:
  if $selection == "exact" or $selection == "storage" or $selection == "label" or $selection == "all" then
    ["backend", "edge"] +
    if $include_system_network then ["podman"] else [] end
  else
    null
  end;

def network_resource_name($network):
  if $network == "podman" then $network else $resource_prefix + $network end;

# Podman-target reimports retain target-loss diagnostics without image-grammar rejection.

def healthcheck_services:
  ["auth", "db", "functions", "imgproxy", "kong", "realtime", "rest", "storage", "studio"];

def image_environment_services:
  ["auth", "db", "functions", "imgproxy", "kong", "meta", "realtime", "storage", "studio", "supavisor"];

def image_label_services:
  ["imgproxy", "kong"];

def network_alias_services:
  ["auth", "boundary-peer", "db", "functions", "imgproxy", "kong", "meta", "realtime", "rest", "storage", "studio", "supavisor"];

def alias_networks($service):
  if $service == "boundary-peer" then
    ["edge"]
  elif $service == "kong" then
    ["backend", "edge"]
  else
    ["backend"]
  end;

def container_native_field_occurrences:
  {
    auth: 91,
    "boundary-peer": 92,
    db: 94,
    functions: 93,
    imgproxy: 93,
    kong: 94,
    meta: 92,
    realtime: 91,
    rest: 92,
    storage: 93,
    studio: 93,
    supavisor: 91,
  };

def image_native_field_occurrences:
  {
    auth: 7,
    db: 9,
    functions: 4,
    imgproxy: 9,
    kong: 10,
    meta: 8,
    realtime: 7,
    rest: 6,
    storage: 8,
    studio: 8,
    supavisor: 7,
  };

def sum_values:
  add // 0;

def selected_unusable_bind_mount_count:
  [
    unusable_bind_mounts | to_entries[] |
    select(.key as $service | contains(selected_services; $service)) |
    .value[]
  ] | length;

def selected_image_label_count:
  [image_label_services[] | select(. as $service | contains(selected_services; $service))] | length;

def podman_native_unsupported_occurrences:
  ([selected_services[] | container_native_field_occurrences[.]] | sum_values) +
  ([selected_services[] | image_native_field_occurrences[.] // 0] | sum_values) +
  ((selected_networks | length) * 5) +
  ((selected_volumes | length) * 3) +
  ((selected_services | length) * 2) +
  selected_unusable_bind_mount_count +
  selected_image_label_count;

def import_service_fields($service):
  [["environment", "approximated"]] + [
    ["health_failure_action", "not-promoted"],
    ["infra", "not-promoted"],
    ["logging", "not-promoted"],
    ["namespaces", "not-promoted"],
    ["networks", "approximated"],
    ["resource_controls", "not-promoted"],
    ["restart_policy", "approximated"],
    ["security", "not-promoted"]
  ] + (
    if $service == "db" then
      [["mounts[0]", "approximated"], ["mounts[1]", "not-promoted"]]
    elif $service == "functions" then
      [["mounts[0]", "approximated"], ["mounts[1]", "not-promoted"]]
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
    selected_networks[] |
    tuple("BFP0002"; "network:" + network_resource_name(.); "omitted")
  ] + [
    selected_volumes[] |
    tuple("BFP0002"; "volume:" + $resource_prefix + .; "omitted")
  ] + [
    selected_volumes[] |
    tuple("BFP0002"; "volume:" + $resource_prefix + .; "omitted")
  ] + [
    selected_images[] |
    tuple("BFP0002"; "image:" + .; "omitted")
  ] + [
    image_label_services[] as $service |
    select(contains(selected_services; $service)) |
    tuple("BFP0002"; "images." + service_images[$service] + ".labels"; "omitted")
  ] + [
    selected_networks[] as $network |
    ["driver", "ipam_driver", "native_ipv6_enabled"][] |
    tuple("BFP0002"; "networks." + network_resource_name($network) + "." + .; "omitted")
  ] + [
    selected_services[] as $service |
    ["creation_evidence", "hostname"][] |
    tuple("BFP0002"; "services." + $resource_prefix + $service + "." + .; "omitted")
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
    selected_services[] as $service |
    select(contains(network_alias_services; $service)) |
    alias_networks($service)[] as $network |
    tuple(
      "BFP0003";
      "services." + $resource_prefix + $service + ".networks." + $resource_prefix + $network + ".aliases";
      "approximated"
    )
  ] + [
    observed_mounts | to_entries[] |
    .key as $service |
    select(contains(selected_services; $service)) |
    .value[] as $index |
    ["options", "propagation"][] |
    tuple(
      "BFP0003";
      "services." + $resource_prefix + $service + ".mounts[" + ($index | tostring) + "]." + .;
      "not-promoted"
    )
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
    selected_networks[] as $network |
    tuple("BFP0003"; "networks." + network_resource_name($network) + ".internal"; "approximated")
  ] + [
    selected_networks[] as $network |
    tuple("BFP0003"; "networks." + network_resource_name($network) + ".ipam"; "approximated")
  ] + [
    selected_images[] as $image |
    ["architecture", "created", "digest", "manifest_type", "operating_system"][] |
    tuple("BFP0003"; "images." + $image + "." + .; "not-promoted")
  ] + [
    image_environment_services[] as $service |
    select(contains(selected_services; $service)) |
    tuple("BFP0003"; "images." + service_images[$service] + ".environment"; "not-promoted")
  ] + [
    selected_volumes[] as $volume |
    ["anonymous", "created_at", "driver", "gid", "uid"][] |
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
  if $input == "podman" then
    []
  else
    [
      selected_services[] |
      tuple("BFC0009"; "services." + $resource_prefix + . + ".image"; null)
    ]
  end;

def compose_network_diagnostics:
  if $input == "podman" then
    [tuple("BFC0007"; "networks." + $resource_prefix + "backend.ipam.config"; null)] +
    [tuple("BFC0007"; "networks." + $resource_prefix + "edge.ipam.config"; null)] +
    [
      tuple("BFC0007"; "networks." + $resource_prefix + "edge.internal"; null),
      tuple("BFC0007"; "networks." + $resource_prefix + "edge.labels"; null)
    ] +
    if $include_system_network then
      [tuple("BFC0007"; "networks.podman.ipam.config"; null)]
    else
      []
    end
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

def observed_podman_environment_fields:
  {
    auth: [
      "API_EXTERNAL_URL",
      "GOTRUE_API_HOST",
      "GOTRUE_API_PORT",
      "GOTRUE_DB_DATABASE_URL",
      "GOTRUE_DB_DRIVER",
      "GOTRUE_DB_MIGRATIONS_PATH",
      "GOTRUE_DISABLE_SIGNUP",
      "GOTRUE_EXTERNAL_EMAIL_ENABLED",
      "GOTRUE_JWT_ADMIN_ROLES",
      "GOTRUE_JWT_AUD",
      "GOTRUE_JWT_DEFAULT_GROUP_NAME",
      "GOTRUE_JWT_EXP",
      "GOTRUE_JWT_SECRET",
      "GOTRUE_MAILER_AUTOCONFIRM",
      "GOTRUE_SITE_URL",
      "HOME",
      "HOSTNAME",
      "PATH",
      "container"
    ],
    db: [
      "GRN_PLUGINS_DIR",
      "HOME",
      "HOSTNAME",
      "LANG",
      "LANGUAGE",
      "LC_ALL",
      "LOCALE_ARCHIVE",
      "PATH",
      "PGDATA",
      "POSTGRES_DB",
      "POSTGRES_HOST",
      "POSTGRES_INITDB_ARGS",
      "POSTGRES_PASSWORD",
      "POSTGRES_USER",
      "container"
    ],
    functions: [
      "HOME",
      "HOSTNAME",
      "JWT_SECRET",
      "PATH",
      "SUPABASE_ANON_KEY",
      "SUPABASE_DB_URL",
      "SUPABASE_SERVICE_ROLE_KEY",
      "SUPABASE_URL",
      "VERIFY_JWT",
      "container"
    ],
    imgproxy: [
      "AWS_LWA_ASYNC_INIT",
      "AWS_LWA_INVOKE_MODE",
      "AWS_LWA_READINESS_CHECK_PATH",
      "FONTCONFIG_PATH",
      "HOME",
      "HOSTNAME",
      "IMGPROXY_BIND",
      "IMGPROXY_LOCAL_FILESYSTEM_ROOT",
      "IMGPROXY_MALLOC",
      "IMGPROXY_MAX_SRC_RESOLUTION",
      "IMGPROXY_USE_ETAG",
      "MALLOC_ARENA_MAX",
      "PATH",
      "VIPS_VECTOR",
      "VIPS_WARNING",
      "container"
    ],
    kong: [
      "HOME",
      "HOSTNAME",
      "KONG_DATABASE",
      "KONG_DECLARATIVE_CONFIG",
      "KONG_DNS_ORDER",
      "KONG_PLUGINS",
      "KONG_PREFIX",
      "KONG_STATUS_LISTEN",
      "KONG_VERSION",
      "PATH",
      "container"
    ],
    meta: [
      "CRYPTO_KEY",
      "HOME",
      "HOSTNAME",
      "NODE_VERSION",
      "PATH",
      "PG_META_DB_HOST",
      "PG_META_DB_NAME",
      "PG_META_DB_PASSWORD",
      "PG_META_DB_PORT",
      "PG_META_DB_USER",
      "PG_META_PORT",
      "YARN_VERSION",
      "container"
    ],
    realtime: [
      "API_JWT_SECRET",
      "APP_NAME",
      "DB_AFTER_CONNECT_QUERY",
      "DB_ENC_KEY",
      "DB_HOST",
      "DB_NAME",
      "DB_PASSWORD",
      "DB_PORT",
      "DB_USER",
      "DNS_NODES",
      "ECTO_IPV6",
      "ERL_AFLAGS",
      "HOME",
      "HOSTNAME",
      "LANG",
      "LANGUAGE",
      "LC_ALL",
      "METRICS_JWT_SECRET",
      "MIX_ENV",
      "PATH",
      "PORT",
      "RLIMIT_NOFILE",
      "RUN_JANITOR",
      "SECRET_KEY_BASE",
      "SEED_SELF_HOST",
      "SLOT_NAME_SUFFIX",
      "container"
    ],
    rest: [
      "HOME",
      "HOSTNAME",
      "PATH",
      "PGRST_ADMIN_SERVER_HOST",
      "PGRST_ADMIN_SERVER_PORT",
      "PGRST_DB_ANON_ROLE",
      "PGRST_DB_EXTRA_SEARCH_PATH",
      "PGRST_DB_SCHEMAS",
      "PGRST_DB_URI",
      "PGRST_JWT_SECRET",
      "container"
    ],
    storage: [
      "ANON_KEY",
      "AUTH_JWT_SECRET",
      "DATABASE_URL",
      "ENABLE_IMAGE_TRANSFORMATION",
      "FILE_SIZE_LIMIT",
      "FILE_STORAGE_BACKEND_PATH",
      "GLOBAL_S3_BUCKET",
      "HOME",
      "HOSTNAME",
      "IMGPROXY_URL",
      "NODE_VERSION",
      "PATH",
      "POSTGREST_URL",
      "REGION",
      "REQUEST_ALLOW_X_FORWARDED_PATH",
      "SERVICE_KEY",
      "STORAGE_BACKEND",
      "STORAGE_PUBLIC_URL",
      "TENANT_ID",
      "VERSION",
      "YARN_VERSION",
      "container"
    ],
    studio: [
      "AUTH_JWT_SECRET",
      "DEFAULT_ORGANIZATION_NAME",
      "DEFAULT_PROJECT_NAME",
      "ENABLED_FEATURES_LOGS_ALL",
      "HOME",
      "HOSTNAME",
      "NODE_VERSION",
      "PATH",
      "PG_META_CRYPTO_KEY",
      "PNPM_HOME",
      "PORT",
      "POSTGRES_PASSWORD",
      "POSTGRES_USER_READ_WRITE",
      "STUDIO_PG_META_URL",
      "SUPABASE_ANON_KEY",
      "SUPABASE_PUBLIC_URL",
      "SUPABASE_SERVICE_KEY",
      "SUPABASE_URL",
      "YARN_VERSION",
      "container"
    ],
    supavisor: [
      "API_JWT_SECRET",
      "CLUSTER_POSTGRES",
      "DATABASE_URL",
      "ECTO_IPV6",
      "ERL_AFLAGS",
      "HOME",
      "HOSTNAME",
      "LANG",
      "LANGUAGE",
      "LC_ALL",
      "METRICS_JWT_SECRET",
      "MIX_ENV",
      "PATH",
      "POOLER_DEFAULT_POOL_SIZE",
      "POOLER_MAX_CLIENT_CONN",
      "POOLER_POOL_MODE",
      "POOLER_TENANT_ID",
      "PORT",
      "POSTGRES_DB",
      "POSTGRES_HOST",
      "POSTGRES_PASSWORD",
      "POSTGRES_PORT",
      "REGION",
      "RLIMIT_NOFILE",
      "SECRET_KEY_BASE",
      "VAULT_ENC_KEY",
      "container"
    ]
  };

# Generated Podman artifacts have route-specific exact network-loss subjects.

def generated_podman_network_subjects:
  if $input == "compose" then
    [
      "networks." + $resource_prefix + "backend.internal",
      "networks." + $resource_prefix + "backend.labels"
    ] +
    ["networks." + $resource_prefix + "edge.runtime_name"]
  elif $input == "quadlet" then
    [
      "networks." + $resource_prefix + "backend.internal",
      "networks." + $resource_prefix + "backend.ipam_configs",
      "networks." + $resource_prefix + "backend.labels"
    ]
  else
    null
  end;

def generated_podman_diagnostics($include_healthchecks):
  ([
    generated_podman_network_subjects[] |
    tuple("BFP0007"; .; "omitted")
  ] + [
    selected_volumes[] |
    tuple("BFP0007"; "volumes." + $resource_prefix + . + ".settings"; "omitted")
  ] + [
    observed_podman_environment_fields | to_entries[] |
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

# Podman-origin and generated-artifact diagnostics remain separate exact contracts.

def podman_diagnostics:
  ([
    selected_networks[] as $network |
    ["internal", "ipam_configs", "labels"][] |
    tuple("BFP0007"; "networks." + network_resource_name($network) + "." + .; "omitted")
  ] + [
    selected_volumes[] |
    tuple("BFP0007"; "volumes." + $resource_prefix + . + ".settings"; "omitted")
  ] + [
    observed_podman_environment_fields | to_entries[] |
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

def quadlet_network_alias_diagnostics:
  if $output == "quadlet" and
    ($input == "podman" or $input == "compose") and
    contains(selected_services; "kong")
  then
    [
      tuple(
        "BFQ0003";
        "services." + $resource_prefix + "kong.networks";
        null
      )
    ]
  else
    []
  end;

def expected_diagnostics:
  if selected_services == null or selected_volumes == null or selected_networks == null then
    null
  elif $input == "podman" and $output == "compose" then
    import_diagnostics + compose_output_diagnostics
  elif $input == "podman" and $output == "quadlet" then
    import_diagnostics + quadlet_network_alias_diagnostics
  elif $input == "podman" and $output == "podman" then
    import_diagnostics + podman_diagnostics
  elif $input == "compose" and $output == "podman" then
    generated_podman_diagnostics(false)
  elif $input == "quadlet" and $output == "podman" then
    generated_podman_diagnostics(true) + quadlet_dependency_diagnostics
  elif $input == "quadlet" and $output == "compose" then
    compose_output_diagnostics
  elif $input == "compose" and $output == "compose" then
    compose_output_diagnostics
  elif ($input == "compose" and $output == "quadlet") or
      ($input == "quadlet" and $output == "quadlet") then
    quadlet_network_alias_diagnostics
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


def import_service_outcomes($service):
  ["approximate"] + [
    "unsupported",
    "approximate",
    "unsupported",
    "unsupported"
  ] + (
    if $service == "db" then
      ["approximate", "unsupported"]
    elif $service == "functions" then
      ["approximate", "unsupported"]
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
  expected_diagnostics as $diagnostics |
  if $diagnostics == null then
    null
  else
    ([
      $diagnostics[] |
      select(.code == "BFC0009" or (.code == "BFP0003" and .decision == "approximated"))
    ] | length) as $diagnostic_approximate |
    ([
      $diagnostics[] |
      select(.code != "BFC0009" and (.code != "BFP0003" or .decision != "approximated"))
    ] | length) as $diagnostic_unsupported |
    ([$diagnostics[] | select(.code == "BFP0002")] | length) as $native_diagnostic_count |
    {
      approximate: (
        $diagnostic_approximate +
        if $input == "podman" and contains(selected_services; "kong") then 1 else 0 end
      ),
      unsupported: (
        $diagnostic_unsupported +
        if $input == "podman" then
          podman_native_unsupported_occurrences - $native_diagnostic_count
        else
          0
        end
      ),
      invalid: 0,
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


if $emit_expected == "fidelity" then
  expected_success_fidelity
elif $emit_expected == true then
  expected_diagnostics
else
  expected_diagnostics as $expected |
  $expected != null and
  success_contract($expected)
end
