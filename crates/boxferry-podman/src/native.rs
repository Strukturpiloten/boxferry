//! Private tolerant subsets of Podman's inspect JSON shapes.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct ContainerInspect {
    pub(crate) id: String,
    pub(crate) image: String,
    #[serde(default)]
    pub(crate) image_name: Option<String>,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) is_infra: bool,
    #[serde(default)]
    pub(crate) is_service: bool,
    #[serde(default)]
    pub(crate) dependencies: Vec<String>,
    #[serde(default)]
    pub(crate) pod: Option<String>,
    #[serde(default)]
    pub(crate) config: Option<ContainerConfig>,
    #[serde(default)]
    pub(crate) host_config: BTreeMap<String, Value>,
    #[serde(default)]
    pub(crate) mounts: Vec<InspectMount>,
    #[serde(default)]
    pub(crate) network_settings: Option<NetworkSettings>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct ContainerConfig {
    #[serde(default)]
    pub(crate) image: Option<String>,
    #[serde(default)]
    pub(crate) env: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) cmd: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) create_command: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) user: Option<String>,
    #[serde(default)]
    pub(crate) working_dir: Option<String>,
    #[serde(flatten)]
    pub(crate) other: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct InspectMount {
    #[serde(rename = "Type")]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) source: String,
    pub(crate) destination: String,
    #[serde(default)]
    pub(crate) mode: String,
    #[serde(default)]
    pub(crate) driver: String,
    #[serde(default)]
    pub(crate) options: Vec<String>,
    #[serde(default = "true_value", rename = "RW")]
    pub(crate) rw: bool,
    #[serde(default)]
    pub(crate) propagation: String,
    #[serde(default)]
    pub(crate) sub_path: String,
}

const fn true_value() -> bool {
    true
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct NetworkSettings {
    #[serde(default)]
    pub(crate) ports: BTreeMap<String, Option<Vec<PortBinding>>>,
    #[serde(default)]
    pub(crate) networks: BTreeMap<String, AdditionalNetwork>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct PortBinding {
    #[serde(default)]
    pub(crate) host_ip: String,
    pub(crate) host_port: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct AdditionalNetwork {
    #[serde(default)]
    pub(crate) aliases: Vec<String>,
    #[serde(flatten)]
    pub(crate) other: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct ImageInspect {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) user: Option<String>,
    #[serde(default)]
    pub(crate) config: Option<ImageConfig>,
    #[serde(flatten)]
    pub(crate) other: BTreeMap<String, Value>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct ImageConfig {
    #[serde(default)]
    pub(crate) env: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) cmd: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) user: Option<String>,
    #[serde(default)]
    pub(crate) working_dir: Option<String>,
    #[serde(flatten)]
    pub(crate) other: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
pub(crate) struct NetworkInspect {
    pub(crate) name: String,
    #[serde(flatten)]
    pub(crate) other: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct VolumeInspect {
    pub(crate) name: String,
    #[serde(flatten)]
    pub(crate) other: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct PodInspect {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) containers: Vec<PodContainer>,
    #[serde(default)]
    pub(crate) create_command: Option<Vec<String>>,
    #[serde(flatten)]
    pub(crate) other: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
pub(crate) struct PodContainer {
    #[serde(rename = "Id")]
    pub(crate) id: String,
}
