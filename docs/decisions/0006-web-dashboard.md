# 0006: The web dashboard reads files; it does not talk to the swarm

Status: accepted (2026-10-12)

## Context

docs/architecture.md lists a web dashboard after the MVP: refrigerator
cards, incident history, development controls. A browser reaches a Myrmic
swarm through a gateway, and a gateway offers a cell two things
(docs/myrmic-notes.md, section 16):

- **Static files** from the cell's blob store. Read-only for the browser.
- **A message API**: the browser opens a session and posts commands and
  events; answers a cell sends to the session arrive on a stream.

What was measured before choosing:

- The message API is not limited to the cell that mounts it. Once one cell
  mounts it, an HTTP client can send any command to any cell and publish
  any event.
- The gateway embedded in a runtime listens on every interface and has no
  login.
- The blob store never deletes content, and stores equal content again on
  every write. A file rewritten on every update, which is what the Myrmic
  tutorials do, grows the database for good.
- A command from a cell to a cell that is not deployed is kept and delivered
  when the cell returns.

So the two ways trade against each other: files cost disk space for every
update; the message API costs nothing and lets everyone who can reach the
port change limits and forge readings (docs/security.md, what is at stake).

## Decision

1. **A fourth cell, `dashboard-cell`, as an adapter that owns no truth.** It
   asks the asset cell (`get_status`) and the risk monitor (`get_incidents`)
   on a timer, as any caller would, and holds nothing that is not rebuilt
   from its init payload and their answers. The three other cells are
   unchanged.
2. **The page gets its data from files.** Each answer is stored, as it
   arrived, beside the page; the page polls. A view-only deployment mounts
   no message API at all, so the gateway has no path into the swarm.
3. **The message API is mounted only with `dev_controls`**, for the page's
   scenario buttons, and that setting is off by default and absent from the
   `prod` profile. The buttons post `simulate_scenario` straight to the
   sensor cell, which keeps its own `dev_mode` gate.
4. **Write as little as the page can do with.** The incident list is written
   only when it says something new (it is compared without its `as_of`).
   The status snapshot is written on every refresh: with readings arriving
   it is new each time, and its `as_of` is the page's proof that the
   dashboard and the asset cell are alive. The refresh period is in the
   init payload: 30 s under `prod`, 2 s under `dev`.
5. **Silence is shown, not hidden.** A cell that leaves three refreshes
   unanswered has its file removed, and the page shows `No answer` for a
   missing file. A cell that does not answer is asked once and then every
   30 refreshes, not on every refresh, so that its return is not met by a
   queue of queries. The page also checks each snapshot's age against the
   gateway's clock (the `Date` header), in case the dashboard cell itself
   has stopped.
6. **One page, no dependencies.** `assets/index.html` holds its CSS and
   JavaScript, is baked into the cell with `include_bytes!`, and loads
   nothing else. No framework, no build step, no `serde_json` in the cell.

## Consequences

- A `prod` dashboard cannot be used to change anything. Acknowledging and
  changing limits stay with `chillguard`.
- The dashboard costs disk space while it runs: about 0.8 KiB of blob
  content per status snapshot that is never reclaimed, 2.3 MiB a day at a
  30 s refresh and 35 MiB a day at 2 s; on disk the database grew by about
  7.7 kB per refresh over a short run, with how much of that compaction
  returns unknown (docs/myrmic-notes.md, section 16). This is the price of
  point 2 on Myrmic 0.7.0 and the reason the refresh period is
  configurable and the cell can be left out of a deployment.
- The page is up to one refresh behind, plus its own polling interval.
- Each refresh adds a `status sent` line to the asset's log and an
  `incidents sent` line to the monitor's.
- The page is a second reader of the wire contract (0004), in JavaScript,
  with no compiler to hold it to the types. The `dashboard` scenario of
  `make e2e` reads the same files the page does.
- The dashboard's init payload names the other cells, so it has to follow
  their `SRN_PREFIX`.

## Alternatives considered

- **The page asks the cells itself over the message API.** No storage, no
  polling when nobody looks, always current. Rejected because it needs the
  message API in every deployment (first measured fact above).
- **A read model built from events** (`reading_accepted`, `status_changed`,
  `alert_raised`, `incident_recovered`), as in the Smart Greenhouse
  tutorial. It cannot show the incident history from before the dashboard
  was deployed, nor the status before the first change, nor the exposure of
  an open incident, which no event carries as it grows; queries would be
  needed anyway.
- **Writing both answers on every refresh.** Simplest, and the incident
  list's `as_of` would prove the monitor alive. It doubled the growth for a
  file that almost never changes; point 5 covers the monitor's silence
  instead.
