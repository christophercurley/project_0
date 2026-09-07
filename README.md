# Daymark / DMK

Milestones 1–3: pure Rust domain rules, SQLite persistence with durable audit,
and an Axum API with accounts, sessions and narrow administration.
The product name is provisionally approved. See [the approved implementation
plan](docs/IMPLEMENTATION_PLAN.md), [domain coverage](docs/MILESTONE_1.md), and
[persistence design and coverage](docs/MILESTONE_2.md), and
[API/security design and local invocation](docs/MILESTONE_3.md).

## Local validation

A Rust toolchain with rustfmt, Clippy and a C compiler for bundled SQLite is
required. Cargo fetches locked dependencies initially. Tests create temporary
local databases; no external database service, browser or container is required.

```text
cargo build --workspace --all-targets --locked
cargo test --workspace --locked
cargo test --workspace --release --locked
cargo test -p daymark-domain --no-default-features --locked
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
git diff --check
```

`daymark-api` runs the JSON API and provides interactive administrator bootstrap
and local emergency sole-admin password recovery commands. See the Milestone 3
document for the recovery workflow, local invocation, cookie/CSRF
requirements, limits and the one-application-process storage contract.
Frontend, Docker and operational tooling belong to later milestones.

## Domain boundary

`Ledger` is a deterministic in-memory reference model with private state. Its
mutation methods validate candidate effective state and commit only successful
changes. Callers supply a trusted user ID and UTC audit timestamp; this library
does not authenticate callers or grant administrator privileges. The later
application must authorize calendar changes and bind personal user IDs to sessions.

Hours are nonnegative integers. Source grants/uses require a positive amount;
adjustments use an explicit nonzero signed integer. Work can carry an explicit
zero Comp credit, with optional integer duration and multiplier context. Nothing
calculates credit from that context. Dates are validated Gregorian calendar dates.

Each source has explicit included dates, stable owner-scoped ID and revision.
Generated effects are projections, not separately editable records. Holiday
conversion references every active supporting source revision. It persists while
any support remains. Every accepted change retains before/after sources and
affected-year snapshots, including changed shared-support relationships.

Snapshots use annual net totals, including future dates. Multi-day usage produces
one deduction. Global calendars produce ten explainable entitlement effects;
unconfigured years are explicitly identified. Deletion history never contributes
to effective totals. Date corrections cannot move a holiday referenced by any
active user's holiday records; prior configuration remains in global history.

`daymark-persistence::Store::open(path)` initializes or checks versioned SQLite
migrations. Store operations preserve domain semantics with real transactions,
ownership constraints, revision checks and persistent audit history. The caller
supplies trusted owner IDs and times, and must authorize global calendar changes.
See the Milestone 2 document for the API/storage contract and deferred boundaries.
