//! The cell's deploy-time configuration.

use myrmic_sdk::{Message, Sri, String, Vec};
use serde::{Deserialize, Serialize};

use crate::routes::{Route, Source};

/// The most refrigerators one dashboard shows. Every refresh sends two
/// queries per refrigerator from one handler; this keeps a refresh small.
pub(crate) const MAX_REFRIGERATORS: usize = 8;

/// The shortest refresh period. Each query is a command the asset or the
/// monitor handles between two readings; faster than this only queues work
/// in front of them.
pub(crate) const MIN_REFRESH_MS: u64 = 1_000;

/// The init payload: `myrmic deploy ... --init-file config/dashboard-cell.<profile>.json`.
///
/// Unknown fields are refused, so a misspelt key fails the deployment
/// instead of silently leaving a setting at its default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Message)]
#[serde(deny_unknown_fields)]
pub(crate) struct DashboardInit {
    /// The refrigerators to show, one card each, in this order.
    pub refrigerators: Vec<Refrigerator>,
    /// Time between two rounds of queries, in milliseconds.
    pub refresh_ms: u64,
    /// Whether the page offers the simulator's scenarios. Off by default:
    /// with it on, the gateway accepts commands for the swarm over HTTP
    /// (docs/security.md).
    #[serde(default)]
    pub dev_controls: bool,
}

/// The cells of one refrigerator, by the names they were deployed under
/// (`SRN_PREFIX` included).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Refrigerator {
    /// The sensor cell: the target of the page's development controls.
    pub sensor: String,
    /// The asset cell: asked for the status snapshot.
    pub asset: String,
    /// The risk monitor cell: asked for the incidents.
    pub monitor: String,
}

impl DashboardInit {
    /// Refuses a configuration the cell cannot run with and resolves the
    /// cells' names. An error here fails the deployment.
    pub(crate) fn routes(&self) -> myrmic_sdk::Result<Vec<Route>> {
        if self.refrigerators.is_empty() {
            return Err("refrigerators must not be empty");
        }
        if self.refrigerators.len() > MAX_REFRIGERATORS {
            return Err("refrigerators lists more than 8 entries");
        }
        if self.refresh_ms < MIN_REFRESH_MS {
            return Err("refresh_ms must be at least 1000");
        }
        let mut routes: Vec<Route> = Vec::new();
        for refrigerator in &self.refrigerators {
            Sri::of_path(&refrigerator.sensor).map_err(|_| "sensor is not a valid cell name")?;
            let asset =
                Sri::of_path(&refrigerator.asset).map_err(|_| "asset is not a valid cell name")?;
            let monitor = Sri::of_path(&refrigerator.monitor)
                .map_err(|_| "monitor is not a valid cell name")?;
            // An answer is filed under the refrigerator whose cell sent it,
            // so one cell cannot stand for two refrigerators.
            let taken = routes
                .iter()
                .any(|other| other.asset.cell == asset || other.monitor.cell == monitor);
            if taken || asset == monitor {
                return Err("a cell is named for more than one role or refrigerator");
            }
            routes.push(Route {
                asset: Source::new(asset),
                monitor: Source::new(monitor),
            });
        }
        Ok(routes)
    }
}

#[cfg(test)]
mod tests {
    use myrmic_sdk::{format, vec};

    use super::*;

    fn refrigerator(prefix: &str) -> Refrigerator {
        Refrigerator {
            sensor: format!("{prefix}sensor-cell"),
            asset: format!("{prefix}fridge-asset-cell"),
            monitor: format!("{prefix}risk-monitor-cell"),
        }
    }

    fn init() -> DashboardInit {
        DashboardInit {
            refrigerators: vec![refrigerator("")],
            refresh_ms: 2_000,
            dev_controls: false,
        }
    }

    #[test]
    fn names_resolve_to_the_sris_the_runtime_routes_on() {
        let routes = init().routes().unwrap();
        assert_eq!(
            routes,
            [Route {
                asset: Source::new(Sri::of_path("fridge-asset-cell").unwrap()),
                monitor: Source::new(Sri::of_path("risk-monitor-cell").unwrap()),
            }]
        );
    }

    #[test]
    fn routes_keep_the_order_of_the_payload() {
        let init = DashboardInit {
            refrigerators: vec![refrigerator("a-"), refrigerator("b-")],
            ..init()
        };
        let routes = init.routes().unwrap();
        assert_eq!(routes.len(), 2);
        assert_eq!(
            routes[1].asset.cell,
            Sri::of_path("b-fridge-asset-cell").unwrap()
        );
    }

    #[test]
    fn a_configuration_the_cell_cannot_run_with_is_refused() {
        let none = DashboardInit {
            refrigerators: Vec::new(),
            ..init()
        };
        assert_eq!(none.routes(), Err("refrigerators must not be empty"));

        let many = DashboardInit {
            refrigerators: (0..=MAX_REFRIGERATORS)
                .map(|n| refrigerator(&format!("f{n}-")))
                .collect(),
            ..init()
        };
        assert_eq!(
            many.routes(),
            Err("refrigerators lists more than 8 entries")
        );

        let fast = DashboardInit {
            refresh_ms: MIN_REFRESH_MS - 1,
            ..init()
        };
        assert_eq!(fast.routes(), Err("refresh_ms must be at least 1000"));

        let mut bad_name = init();
        bad_name.refrigerators[0].monitor = String::from("Risk Monitor");
        assert_eq!(bad_name.routes(), Err("monitor is not a valid cell name"));
    }

    #[test]
    fn one_cell_cannot_stand_for_two_refrigerators() {
        let twice = DashboardInit {
            refrigerators: vec![refrigerator(""), refrigerator("")],
            ..init()
        };
        assert!(twice.routes().is_err());

        let mut same_cell = init();
        same_cell.refrigerators[0].monitor = same_cell.refrigerators[0].asset.clone();
        assert!(same_cell.routes().is_err());
    }

    #[test]
    fn unknown_keys_and_missing_cells_fail_the_deployment() {
        let decode = |json: &str| {
            <myrmic_sdk::Json as myrmic_sdk::Codec>::decode::<DashboardInit>(json.as_bytes())
        };
        let cells = r#"{"sensor":"s","asset":"a","monitor":"m"}"#;
        let ok = decode(&format!(
            r#"{{"refrigerators":[{cells}],"refresh_ms":2000}}"#
        ))
        .unwrap();
        assert!(!ok.dev_controls, "dev_controls must default to off");
        assert!(
            decode(&format!(
                r#"{{"refrigerators":[{cells}],"refresh_ms":2000,"dev_control":true}}"#
            ))
            .is_err()
        );
        assert!(
            decode(r#"{"refrigerators":[{"asset":"a","monitor":"m"}],"refresh_ms":2000}"#).is_err()
        );
    }
}
