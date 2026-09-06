# Daymark / DMK

Milestone 1: a pure Rust domain library and deterministic automated test harness.
The product name is provisionally approved. See [the approved implementation
plan](docs/IMPLEMENTATION_PLAN.md) and [milestone coverage](docs/MILESTONE_1.md).

## Local validation

A Rust toolchain with rustfmt and Clippy is sufficient. There are no external
crate dependencies, network services, database, browser or container requirements.

```text
cargo build --all-targets --locked
cargo test --locked
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
```

The library is not a running web application. Authentication, SQLite, Axum,
frontend, Docker and operational tooling belong to later milestones.

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

Do not treat this in-memory implementation as durable storage or an HTTP security
boundary. Future SQLite work must preserve its semantics with real transactions,
ownership constraints, revision checks and persistent audit history.
