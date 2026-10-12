//! Myrmic handler exports for the cell.
//!
//! Only functions carrying an SDK handler attribute belong here; keep them to
//! decode, persist, emit.

// The SDK's handler attributes expand to `#[unsafe(no_mangle)] extern "C"`
// exports, which the workspace-wide `unsafe_code = "deny"` rejects. The allow
// is confined to this module; nothing here may contain hand-written `unsafe`.
#![allow(unsafe_code)]

use core::time::Duration;

use chillguard_protocol::commands::{GetIncidents, GetStatus, ReplyTo};
use chillguard_protocol::responses::{IncidentList, StatusSnapshot};
use chillguard_protocol::{Envelope, Payload, Timestamp};
use myrmic_sdk::db::store::BlobStore;
use myrmic_sdk::gateway::{self, Fallback};
use myrmic_sdk::{
    Bytes, Callback, Codec, Encoder, Handler, InMemory, Json, Metadata, Result, Vec, format,
};
use serde::Serialize;

use crate::config::DashboardInit;
use crate::routes::{Refresh, Route, SILENT_AFTER, Source, fingerprint};

/// The URL prefix the page is served under, on every gateway of the swarm.
const MOUNT: &str = "/chillguard";
/// The gateway's message API, relative to [`MOUNT`]. Mounted with
/// `dev_controls` only.
const API: &str = "/api";
/// The page, in the cell's asset store.
const INDEX: &str = "/index.html";
/// What the page needs to know about this deployment.
const PAGE_CONFIG: &str = "/config.json";
/// The commands the queries are sent as. A test pins them to the protocol.
const GET_STATUS: &str = "get_status";
const GET_INCIDENTS: &str = "get_incidents";
/// How many incidents per refrigerator the history table shows.
const HISTORY_LIMIT: u16 = 20;
/// The directories of the answers, in the cell's asset store.
const STATUS: &str = "status";
const INCIDENTS: &str = "incidents";

/// The page: one file, no build step, nothing fetched from elsewhere.
static PAGE: &[u8] = include_bytes!("../assets/index.html");

/// The cells to ask, in the order of the init payload, with which of them
/// owe an answer and what the page shows of each. An answer is stored under
/// the position of the cell that sent it.
///
/// In Wasm memory, not in the database: all of it is rebuilt by `init`, which
/// runs in every incarnation, and none of it is worth a write per refresh.
/// Memory is not rolled back with a failed handler, so every handler works
/// on a copy and puts it back as its last step.
static ROUTES: InMemory<Vec<Route>> = InMemory::new(Vec::new());

/// `config.json`: what the page needs to know about this deployment.
#[derive(Serialize)]
struct PageConfig<'a> {
    /// How often the answers change, in milliseconds.
    refresh_ms: u64,
    /// How many refrigerators there are: `status/<n>.json` and
    /// `incidents/<n>.json` exist for `n` below this once the cells answered.
    refrigerators: usize,
    /// Whether the message API is mounted and the page may show its controls.
    dev_controls: bool,
    /// The sensor cell of each refrigerator, for the controls. Empty without
    /// `dev_controls`: the page has no use for the names then.
    sensors: Vec<&'a str>,
}

/// Where the answer of the `index`th refrigerator's cell is served from.
fn data_path(kind: &str, index: usize) -> myrmic_sdk::String {
    format!("/{kind}/{index}.json")
}

/// Runs once per incarnation, before any other handler: on deploy and again
/// on every restart.
#[myrmic_sdk::init]
fn init(md: Metadata, init: DashboardInit) -> Result {
    let routes = match init.routes() {
        Ok(routes) => routes,
        Err(reason) => {
            myrmic_sdk::error!("init refused reason=\"{reason}\"")?;
            return Err(reason);
        }
    };
    // The cell owns no truth, so nothing stored wins over the payload.
    // Answers left by an earlier incarnation are removed so that the page
    // cannot show them as current.
    let assets = gateway::assets(md.id);
    for stale in assets.list()? {
        assets.delete(&stale)?;
    }
    let page_config = PageConfig {
        refresh_ms: init.refresh_ms,
        refrigerators: routes.len(),
        dev_controls: init.dev_controls,
        sensors: if init.dev_controls {
            init.refrigerators
                .iter()
                .map(|refrigerator| refrigerator.sensor.as_str())
                .collect()
        } else {
            Vec::new()
        },
    };
    assets.put(INDEX, PAGE)?;
    assets.put(PAGE_CONFIG, &Json::encode(&page_config)?)?;

    // A path that matches no file is a 404, not the page: the page tells
    // "no answer yet" from the status of its data files.
    let mount = gateway::mount(MOUNT).index(INDEX).fallback(Fallback::None);
    // Without the API path the mount serves files and nothing else: there
    // is no way from HTTP into the swarm.
    let mount = if init.dev_controls {
        mount.api(API)
    } else {
        mount
    };
    mount.bind().map_err(<&'static str>::from)?;

    // Timers die with the incarnation (docs/myrmic-notes.md, section 10), so
    // the interval is armed on every init and nothing is left over to cancel.
    let period = Duration::from_millis(init.refresh_ms);
    let _ = myrmic_sdk::interval(Callback::of::<tick>(), period).build()?;
    myrmic_sdk::info!(
        "dashboard-cell started id={} mount={MOUNT} refrigerators={} refresh_ms={} \
         dev_controls={}",
        md.id,
        routes.len(),
        init.refresh_ms,
        init.dev_controls,
    )?;
    ROUTES.with(|slot| *slot = routes)
}

/// One refresh for one cell: asks it, waits for it, or withdraws what the
/// page shows of it.
fn refresh<T: Encoder>(
    assets: &BlobStore,
    source: &mut Source,
    file: &str,
    command: &str,
    query: &T,
) -> Result {
    match source.refresh() {
        Refresh::Wait => return Ok(()),
        Refresh::Withdraw => {
            // The page reads a missing file as "no answer". Deleting a path
            // that is not there is not an error.
            assets.delete(file)?;
            myrmic_sdk::warn!(
                "answer withdrawn cell={} file={file} reason=no_answer refreshes={SILENT_AFTER}",
                source.cell
            )?;
            return Ok(());
        }
        Refresh::AskAgain => {
            myrmic_sdk::warn!(
                "query repeated cell={} command={command} reason=no_answer",
                source.cell
            )?;
        }
        Refresh::Ask => {}
    }
    if let Err(error) = myrmic_sdk::send(source.cell, command, query) {
        source.not_asked();
        myrmic_sdk::warn!(
            "query not sent cell={} command={command} reason=\"{error}\"",
            source.cell
        )?;
    }
    Ok(())
}

/// The periodic handler: one round of queries. The answers arrive as the
/// commands `status_snapshot` and `incident_list`.
#[myrmic_sdk::cmd]
fn tick(md: Metadata) -> Result {
    let reply_to = |name| ReplyTo::new(name).map_err(|_| "invalid reply name");
    let get_status = Envelope::new(GetStatus {
        reply_to: reply_to(<status_snapshot as Handler>::NAME)?,
    });
    let get_incidents = Envelope::new(GetIncidents {
        limit: HISTORY_LIMIT,
        reply_to: reply_to(<incident_list as Handler>::NAME)?,
    });
    let assets = gateway::assets(md.id);
    let mut routes = ROUTES.with(|routes| routes.clone())?;
    for (index, route) in routes.iter_mut().enumerate() {
        let file = data_path(STATUS, index);
        refresh(&assets, &mut route.asset, &file, GET_STATUS, &get_status)?;
        let file = data_path(INCIDENTS, index);
        refresh(
            &assets,
            &mut route.monitor,
            &file,
            GET_INCIDENTS,
            &get_incidents,
        )?;
    }
    ROUTES.with(|slot| *slot = routes)
}

/// Stores an answer for the page, as it arrived, unless the page shows the
/// same content already (`content` is what counts as the same).
///
/// Never fails on account of the answer: a command that returns an error is
/// redelivered without end (docs/myrmic-notes.md, section 10), and an answer
/// is replaced by the next one a refresh later. Whatever cannot be used is
/// logged and dropped, and the cell is treated as not having answered.
fn answer<T: Payload>(
    md: Metadata,
    kind: &str,
    payload: &[u8],
    source: fn(&mut Route) -> &mut Source,
    content: fn(T, &[u8]) -> Result<u64>,
) -> Result {
    let mut routes = ROUTES.with(|routes| routes.clone())?;
    // Only the cells this dashboard asks may fill its page; anyone on the
    // swarm can send a command of this name.
    let asked = routes
        .iter_mut()
        .map(source)
        .enumerate()
        .find(|(_, source)| source.cell == md.sender);
    let Some((index, source)) = asked else {
        myrmic_sdk::warn!(
            "answer ignored cell={} kind={kind} reason=not_a_configured_cell",
            md.sender
        )?;
        return Ok(());
    };
    let content = Envelope::<T>::decode(payload)
        .map_err(|error| error.as_str())
        .and_then(|envelope| content(envelope.body, payload));
    let content = match content {
        Ok(content) => content,
        Err(reason) => {
            myrmic_sdk::warn!(
                "answer ignored cell={} kind={kind} reason=\"{reason}\"",
                md.sender
            )?;
            return Ok(());
        }
    };
    let file = data_path(kind, index);
    let resumed = source.answered();
    if source.show(content)
        && let Err(error) = gateway::assets(md.id).put(&file, payload)
    {
        source.not_shown();
        myrmic_sdk::warn!(
            "answer not stored cell={} file={file} reason=\"{error}\"",
            md.sender
        )?;
    }
    if resumed {
        myrmic_sdk::info!("answers resumed cell={} file={file}", md.sender)?;
    }
    ROUTES.with(|slot| *slot = routes)
}

/// An asset cell's answer to `get_status`.
///
/// Written every time, although the blob store keeps every version: the
/// snapshot's `as_of` is how the page knows that this cell and the asset
/// cell are alive, and with readings arriving the content is new anyway.
#[myrmic_sdk::cmd]
fn status_snapshot(md: Metadata, payload: Bytes) -> Result {
    answer::<StatusSnapshot>(
        md,
        STATUS,
        &payload,
        |route| &mut route.asset,
        |_, payload| Ok(fingerprint(payload)),
    )
}

/// A risk monitor's answer to `get_incidents`.
///
/// Written when the incidents or their acknowledgements changed, which
/// outside an open incident is rarely: `as_of` alone is not a change.
#[myrmic_sdk::cmd]
fn incident_list(md: Metadata, payload: Bytes) -> Result {
    answer::<IncidentList>(
        md,
        INCIDENTS,
        &payload,
        |route| &mut route.monitor,
        |list, _| incidents_content(list),
    )
}

/// What an incident list says, apart from when it was said.
fn incidents_content(mut list: IncidentList) -> Result<u64> {
    list.as_of = Timestamp::from_millis(0);
    Ok(fingerprint(&Json::encode(&list)?))
}

#[cfg(test)]
mod tests {
    use chillguard_protocol::Kind;

    use super::*;

    /// The names in this file are literals because the handler attributes
    /// need them to be; this ties the ones that can be tied to the protocol.
    #[test]
    fn wire_names_match_the_protocol() {
        assert_eq!(GetStatus::KIND, Kind::Command(GET_STATUS));
        assert_eq!(GetIncidents::KIND, Kind::Command(GET_INCIDENTS));
        assert_eq!(StatusSnapshot::KIND, Kind::Response);
        assert_eq!(IncidentList::KIND, Kind::Response);
    }

    /// The answers come back under the handlers' own names, which must be
    /// names the protocol lets a caller ask for.
    #[test]
    fn reply_names_are_valid_and_the_page_size_is_allowed() {
        ReplyTo::new(<status_snapshot as Handler>::NAME).unwrap();
        ReplyTo::new(<incident_list as Handler>::NAME).unwrap();
        let query = GetIncidents {
            limit: HISTORY_LIMIT,
            reply_to: ReplyTo::new(<incident_list as Handler>::NAME).unwrap(),
        };
        query.validate().unwrap();
    }

    #[test]
    fn answers_are_served_per_refrigerator() {
        assert_eq!(data_path(STATUS, 0), "/status/0.json");
        assert_eq!(data_path(INCIDENTS, 3), "/incidents/3.json");
    }

    #[test]
    fn an_incident_list_is_the_same_whenever_it_was_assembled() {
        let list = |as_of, truncated| IncidentList {
            refrigerator_id: chillguard_protocol::RefrigeratorId::new("fridge-1"),
            incidents: Vec::new(),
            acknowledgements: Vec::new(),
            truncated,
            as_of: Timestamp::from_millis(as_of),
        };
        let content = incidents_content(list(1_000, false)).unwrap();
        assert_eq!(incidents_content(list(2_000, false)).unwrap(), content);
        assert_ne!(incidents_content(list(1_000, true)).unwrap(), content);
    }
}
