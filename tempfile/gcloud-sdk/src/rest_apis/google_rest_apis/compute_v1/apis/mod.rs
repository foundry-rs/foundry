use std::error;
use std::fmt;

#[derive(Debug, Clone)]
pub struct ResponseContent<T> {
    pub status: reqwest::StatusCode,
    pub content: String,
    pub entity: Option<T>,
}

#[derive(Debug)]
pub enum Error<T> {
    Reqwest(reqwest::Error),
    Serde(serde_json::Error),
    Io(std::io::Error),
    ResponseError(ResponseContent<T>),
}

impl<T> fmt::Display for Error<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (module, e) = match self {
            Error::Reqwest(e) => ("reqwest", e.to_string()),
            Error::Serde(e) => ("serde", e.to_string()),
            Error::Io(e) => ("IO", e.to_string()),
            Error::ResponseError(e) => ("response", format!("status code {}", e.status)),
        };
        write!(f, "error in {}: {}", module, e)
    }
}

impl<T: fmt::Debug> error::Error for Error<T> {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        Some(match self {
            Error::Reqwest(e) => e,
            Error::Serde(e) => e,
            Error::Io(e) => e,
            Error::ResponseError(_) => return None,
        })
    }
}

impl<T> From<reqwest::Error> for Error<T> {
    fn from(e: reqwest::Error) -> Self {
        Error::Reqwest(e)
    }
}

impl<T> From<serde_json::Error> for Error<T> {
    fn from(e: serde_json::Error) -> Self {
        Error::Serde(e)
    }
}

impl<T> From<std::io::Error> for Error<T> {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub fn urlencode<T: AsRef<str>>(s: T) -> String {
    ::url::form_urlencoded::byte_serialize(s.as_ref().as_bytes()).collect()
}

pub fn parse_deep_object(prefix: &str, value: &serde_json::Value) -> Vec<(String, String)> {
    if let serde_json::Value::Object(object) = value {
        let mut params = vec![];

        for (key, value) in object {
            match value {
                serde_json::Value::Object(_) => params.append(&mut parse_deep_object(
                    &format!("{}[{}]", prefix, key),
                    value,
                )),
                serde_json::Value::Array(array) => {
                    for (i, value) in array.iter().enumerate() {
                        params.append(&mut parse_deep_object(
                            &format!("{}[{}][{}]", prefix, key, i),
                            value,
                        ));
                    }
                }
                serde_json::Value::String(s) => {
                    params.push((format!("{}[{}]", prefix, key), s.clone()))
                }
                _ => params.push((format!("{}[{}]", prefix, key), value.to_string())),
            }
        }

        return params;
    }

    unimplemented!("Only objects are supported with style=deepObject")
}

pub mod accelerator_types_api;
pub mod addresses_api;
pub mod autoscalers_api;
pub mod backend_buckets_api;
pub mod backend_services_api;
pub mod disk_types_api;
pub mod disks_api;
pub mod external_vpn_gateways_api;
pub mod firewall_policies_api;
pub mod firewalls_api;
pub mod forwarding_rules_api;
pub mod global_addresses_api;
pub mod global_forwarding_rules_api;
pub mod global_network_endpoint_groups_api;
pub mod global_operations_api;
pub mod global_organization_operations_api;
pub mod global_public_delegated_prefixes_api;
pub mod health_checks_api;
pub mod http_health_checks_api;
pub mod https_health_checks_api;
pub mod image_family_views_api;
pub mod images_api;
pub mod instance_group_managers_api;
pub mod instance_groups_api;
pub mod instance_templates_api;
pub mod instances_api;
pub mod instant_snapshots_api;
pub mod interconnect_attachments_api;
pub mod interconnect_locations_api;
pub mod interconnect_remote_locations_api;
pub mod interconnects_api;
pub mod license_codes_api;
pub mod licenses_api;
pub mod machine_images_api;
pub mod machine_types_api;
pub mod network_attachments_api;
pub mod network_edge_security_services_api;
pub mod network_endpoint_groups_api;
pub mod network_firewall_policies_api;
pub mod networks_api;
pub mod node_groups_api;
pub mod node_templates_api;
pub mod node_types_api;
pub mod packet_mirrorings_api;
pub mod projects_api;
pub mod public_advertised_prefixes_api;
pub mod public_delegated_prefixes_api;
pub mod region_autoscalers_api;
pub mod region_backend_services_api;
pub mod region_commitments_api;
pub mod region_disk_types_api;
pub mod region_disks_api;
pub mod region_health_check_services_api;
pub mod region_health_checks_api;
pub mod region_instance_group_managers_api;
pub mod region_instance_groups_api;
pub mod region_instance_templates_api;
pub mod region_instances_api;
pub mod region_instant_snapshots_api;
pub mod region_network_endpoint_groups_api;
pub mod region_network_firewall_policies_api;
pub mod region_notification_endpoints_api;
pub mod region_operations_api;
pub mod region_security_policies_api;
pub mod region_ssl_certificates_api;
pub mod region_ssl_policies_api;
pub mod region_target_http_proxies_api;
pub mod region_target_https_proxies_api;
pub mod region_target_tcp_proxies_api;
pub mod region_url_maps_api;
pub mod region_zones_api;
pub mod regions_api;
pub mod reservations_api;
pub mod resource_policies_api;
pub mod routers_api;
pub mod routes_api;
pub mod security_policies_api;
pub mod service_attachments_api;
pub mod snapshot_settings_api;
pub mod snapshots_api;
pub mod ssl_certificates_api;
pub mod ssl_policies_api;
pub mod subnetworks_api;
pub mod target_grpc_proxies_api;
pub mod target_http_proxies_api;
pub mod target_https_proxies_api;
pub mod target_instances_api;
pub mod target_pools_api;
pub mod target_ssl_proxies_api;
pub mod target_tcp_proxies_api;
pub mod target_vpn_gateways_api;
pub mod url_maps_api;
pub mod vpn_gateways_api;
pub mod vpn_tunnels_api;
pub mod zone_operations_api;
pub mod zones_api;

pub mod configuration;
