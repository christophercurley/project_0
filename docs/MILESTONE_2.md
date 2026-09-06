# Milestone 2: SQLite persistence and durable audit

## Scope and architecture

`daymark-persistence` is a separate synchronous Rust library using explicit SQL
through rusqlite with bundled SQLite. The workspace defaults include both crates.
The accepted domain model remains the business-rule authority. Its only additions
are `Ledger::restore` and optional Serde value encoding. With default features the
domain still has no external dependencies. No accepted Milestone 1 test changed.

Restore validates a complete current aggregate together, retains original source
revisions and reserved IDs, and starts with empty in-memory history. It does not
replay historical operations, validate a chronological running balance, or create
synthetic audit entries. Validated dates, years and hours use their constructors
when decoded. The adapter never implements a second balance/conversion policy.

The adapter loads only current state, not the entire audit history. Mutations load
one owner's active records and tombstones across years so source ID uniqueness,
old-year debits and new-year credits are all checked. Selected-year reads load
that owner's selected year. Calendar correction internally loads all owners'
active records in the affected year so the domain can check every reference.
Global calendars are shared; no per-user entitlement copies or balance caches
are stored. Generated personal effects are persisted and compared with domain
projections on snapshot/query reads. Historic audit snapshots contain the accepted
before/after balances for accountability, not as a cache of current balances.
These checks include effect ordinals and exact relational support links. Source
mutations also verify the existing effects/supports in every affected year before
writing, so a mutation cannot silently repair drift and hide it behind a newly
appended reconstructed audit snapshot.

## Initialization and migration contract

Call `Store::open(path)` on a local SQLite file in an application-owned persistent
directory. It sets foreign keys, WAL, FULL synchronous durability and a five-second
busy timeout, then initializes/checks migrations under `BEGIN IMMEDIATE`.
Each Store owns exactly one connection; there is no hidden pool or background work.
The future Tokio application must bound connection/blocking-worker concurrency
and run this synchronous adapter outside async executor threads.

Migration `persistence/migrations/001_initial.sql` creates the schema atomically.
`schema_migrations` stores its version and exact SQL (with LF/CRLF normalized for
cross-platform builds). Reopening is idempotent; unknown versions or altered
migration text are rejected. A failed migration rolls back all of its schema and
version writes. Future changes need additional ordered migrations and explicit
payload evolution; never edit an applied migration to upgrade an existing DB.
This milestone has one initial migration and no legacy application DB to upgrade.

Tables use SQLite STRICT mode, parameterized SQL and composite keys:

| Table | Purpose and constraints |
| --- | --- |
| `calendars` | One active complete calendar per year, monotonically increasing expected revision, JSON calendar |
| `holidays` | Stable `(year, id)` identities, unique dates within a year, names; referenced by active source holiday links |
| `sources` | `(owner, id)` head, revision, year, optional holiday ID and JSON Record; NULL payload is a deletion tombstone and permanently reserves the ID |
| `effects` | Current domain-generated personal effects keyed by `(owner, year, ordinal)`; no entitlement duplication |
| `effect_supports` | Relational links for source and shared conversion origins; composite FKs include owner, year and exact source revision |
| `source_history` | Append-only ordered Change payloads, indexed by owner/source, UTC timestamp, FK to permanent source identity |
| `calendar_history` | Append-only complete before/after global calendars, actor/time, unique year/revision |

Unsigned IDs and source revisions use eight-byte big-endian blobs, preserving the
domain's entire u64 range without signed SQLite truncation. Calendar revisions use
positive checked i64 values. Hours inside typed JSON remain exact integers;
SQLite does not sum them or interpret business rules. Included dates, source
metadata and effect/support details are encoded with the domain's Serde format.
Migration version 1 also identifies this stored payload format. The relational
columns support ownership/indexing/FKs; JSON retains the complete typed values.
Do not treat this internal format as an HTTP API contract.

Global entitlements derive directly from the durable calendar on each projection.
An unreferenced date correction therefore needs no rewrite of every user's
ledger. Replacing the ten normalized holiday rows uses deferred foreign keys:
the complete replacement is validated by the domain and must satisfy all active
source references at commit. Name changes preserve source IDs/revisions/effects.

## Atomicity, revisions and concurrency

Every create/edit/delete/calendar mutation acquires `BEGIN IMMEDIATE` before
reading current state. SQLite serializes competing writers across connections
and processes. The second writer loads the first writer's committed state; it
cannot validate against an earlier balance. Domain validation precedes writes.
Source head, all affected-year effects/support links and immutable audit append
commit together. Failure in validation, any SQL statement, or deferred FK checks
at commit causes transaction rollback. Busy/locked failures propagate as storage
errors; callers must not report success or retry using a previously validated
in-memory candidate. Any retry must invoke the full operation again.

Source edit/delete requires the expected current revision and expected current
year. A year-moving edit explicitly supplies the old year, and the domain checks
both old and new annual nets. A stale year returns NotFound; a stale revision on
an active record returns StaleRevision. A deleted record returns NotFound and its
ID cannot be reused. Calendar submissions require `None` for first configuration
or `Some(current_revision)` for a correction; stale submissions are rejected
before changes. This adds concurrency control without changing the domain's
accepted sequential `configure` contract.

Snapshot/query reads use a SQLite read transaction so records, calendar and
effects cannot come from different commits. Shared conversions remain state-based:
the durable effect links every active supporting revision; removing one support
does not assign permanent conversion ownership to another source. Removing the
last support validates reversal of the Floater and independent Comp credits.

## Audit and isolation

Successful domain Change payloads preserve actor/owner, UTC timestamp, complete
before/after Record values, deletion evidence, and both affected-year projections
with generated effects and all conversion support revisions. Global calendar
history preserves original names/dates and replacements. History remains readable
after closing/reopening. Update/delete triggers protect both history tables;
foreign keys preserve permanent source identities. This is current state plus
immutable revisions, not event sourcing or a replay-dependent balance system.

Personal record/history/query/mutation methods always take an owner. Record
lookups and mutations also enforce the selected/current year. History is scoped
by owner/source and intentionally includes both years of deliberate year edits.
No global personal-history API exists. Composite effect-support constraints
prevent cross-owner/year/revision links. `configure` is explicitly privileged but
returns only global configuration results, never private source data.

The caller must bind owner IDs and calendar actors to authenticated/authorized
identities and supply trusted UTC times in Milestone 3. This library is not an
authentication boundary. Accounts, sessions, settings, HTTP, admin endpoints and
frontend remain absent. Filesystem owners can alter SQLite files/triggers; these
audit safeguards protect application operations, not a hostile database operator.

## Acceptance evidence

| Criteria | New durable evidence |
| --- | --- |
| DB-002 | Empty initialization, reopen idempotence, rejected migration drift/future version, failed migration rollback |
| DB-001 (storage portion) | Close/reopen preserves sources, effects, calendars and audit; actual container replacement remains M6 |
| BAL-001–005 | Atomic domain/storage/commit failures; competing balance spends; no partial multi-effect results |
| HOL-001–007, ADMIN-002/003 (persistence portion) | Atomic calendars, global stable identities, stale submissions, referenced name/date corrections across users |
| FLT-001–004, COMBO-001–003 | Durable shared supports, last-support rejection with spent Floater/Comp, edit/delete reconciliation, concurrency |
| AUDIT-001–004 | Immutable reconstructable before/after source and effect history; tombstones; owned retrieval and reopen |
| AUTH-003–005 (repository portion) | Owned lookups, mutations, notes search/history and composite ownership constraints; HTTP identity binding remains M3 |
| YEAR-001–004, PTO-001–004 | Isolated annual nets, explicit opening entries, failed and successful year-moving edits, no rollover |
| MULTI-001/003, LEDGER-001–006 | Domain filtering over durable state, explicit date gaps and one multi-day deduction; UI remains deferred |

Integration tests use isolated tempfile directories, real bundled SQLite files,
independent connections and barriers rather than sleeps. SQL fault triggers force
failures after earlier writes and deferred failures during commit; complete table
dumps, including audit sequences, prove rollback. Tests include two competing
spenders for each manual bucket, stale edits and calendar submissions, concurrent
work-support creation/deletion, cross-user ID collisions including u64::MAX,
cross-year support moves, spent-credit deletion, immutable history, failed calendar
writes, large signed net balances, invalid primitive decoding and effect drift.
A Boolean state-machine oracle covers 64 sequences / 192 transitions with close/
reopen after each attempt, checking rejected-state rollback and another owner.

## Remaining boundaries and risks

The adapter loads an owner's current aggregate and rewrites affected-year effects;
its cost grows with active records. Audit snapshots grow with affected-year ledger
size. This favors clarity for a small application; no production load claim is made.
Request sizes, history pagination, bounded blocking execution, identity allocation,
account foreign keys and authorization belong to Milestone 3. The default domain
build remains independent of SQLite and Serde.

No Docker, backup/restore scripts, container replacement proof or real deployment
has been added. M6 must exercise persistent mounted storage (including WAL/SHM),
consistent SQLite backups and restores on the Linux target. Reopen tests do not
claim power-loss or backup proof. No unresolved product ambiguity was found.

Concurrent first-time opens may return SQLITE_BUSY while switching journal mode
before the migration transaction. The application may retry the entire open;
initialization never reports an accepted partial migration.

## Independent review and final validation

The independent reviewer read the requirements, implementation and tests, then
added three tests in `persistence/tests/independent_review.rs`:

- 32 replacement/deletion cases compare the adapter with the accepted domain,
  including exact durable audit, notes search, colliding owners and reopen.
- Holiday date correction races the first HolidayWork or HolidayUse reference;
  exactly one serializable outcome succeeds.
- Four simultaneous clean initializers produce one intact schema, with full-open
  retry allowed for SQLite's journal-mode contention.

These passed in debug and release. The independent review found no additional
justified defect and changed no production code or existing test.

The implementation agent's diff review reproduced one adapter/domain mismatch:
SQLite's `length(trim(name))` treats embedded NUL as the end of text, rejecting a
nonempty holiday name accepted by the domain. Removed that duplicate validation;
the domain still validates names. A regression verifies exact persistence/reopen.
The reviewer confirmed this remediation. Another test verifies LF/CRLF migration
identity portability. No valid test was weakened or rewritten to obtain a pass.

Final validation on Rust/Cargo 1.97.1:

- `cargo build --workspace --all-targets --locked`: passed.
- `cargo test --workspace --locked --quiet`: 76 tests passed, including all 52
  accepted Milestone 1 tests, 2 restore tests and 22 SQLite integration/review tests.
- `cargo test --workspace --release --locked --quiet`: the same 76 tests passed.
- `cargo test -p daymark-domain --no-default-features --locked`: 54 tests passed;
  `cargo tree` confirmed the default domain has no external dependencies.
- Doc tests for both crates passed (no examples).
- `cargo fmt --all --check`: passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `git diff --check` and staged diff checks: passed. Accepted Milestone 1 test
  files are unchanged from master.

Host home-path canonicalization and Git line-ending notices are environmental;
no project compiler or Clippy warnings occurred. These are local Windows/bundled
SQLite results, not a claim of Linux deployment validation.

Work is confined to `milestone-2-persistence`, based on accepted master `462e164`.
Milestone 3 has not begun; no merge to master or real deployment was performed.

## Fresh independent release review (2026-09-06)

Reviewed AGENTS.md, CODEX_RUNBOOK.md, all authoritative documents, the plan and
both milestone reports before inspecting the full domain, adapter, migration and
existing tests. Baseline was `a139730` on `milestone-2-persistence`. This review
attempted to falsify correctness independently of the earlier review's conclusions.

Two fault-injected corruption-handling defects were reproduced with failing tests:

1. Snapshot reads rejected a changed persisted effect, but create/edit/delete
   accepted the same state, overwrote the discrepancy, and appended an audit whose
   before-effects described the reconstructed domain projection rather than the
   persisted effects. Mutations now reject drift in all affected years before any
   durable write. This includes both sides of year-moving edits.
2. Missing or misdirected relational effect supports were never compared with
   generated origins. SQLite foreign keys still passed when a support was deleted
   or redirected to an existing unrelated source in the same owner/year/revision.
   Verification now checks exact support membership/revisions and effect ordinals
   against the domain projection on snapshot/query reads and affected-year writes.

These are fail-closed integrity fixes, not a claim that ordinary Store operations
were observed producing corruption. No business-rule disagreement was reproduced
from valid state. No migration, payload format, domain code or existing test was
changed; the applied migration remains byte-for-byte unchanged.

`persistence/tests/release_review.rs` adds eight substantive tests and one child
process entry point:

- Effect drift rejects create/edit/delete and year moves without durable changes.
- Missing shared supports and an FK-valid unrelated support fail reads and writes.
- Six insertion orders with full-width user/source/holiday IDs match domain
  projections and exact accepted changes after every reopen.
- A valid `i64::MIN` adjustment survives source/effect/audit encoding and reopen;
  deleting it would overflow the net and is rejected without writes.
- Synthetic high-revision heads exercise the signed boundary, `u64::MAX`, edit
  overflow, deletion and permanent ID reservation. These fixtures do not pretend
  to reproduce billions of historical edits.
- A deferred foreign-key failure at calendar commit rolls back calendar rows,
  normalized holidays, sources/effects, audit and sequence state; retry succeeds.
- A separate process times out against a held `BEGIN IMMEDIATE`, leaves no writes,
  and retries on the same Store only after another writer spends the balance; the
  retry rejects the spend from freshly loaded state.
- A separate process exits without Rust destructors after uncommitted source,
  effect, support and audit inserts. Other readers and subsequent reopen see none
  of those writes, and the attempted source ID remains available.

The process tests use pipe handshakes with bounded signal waits. Existing tests
continue to cover concurrent initialization, stale source/calendar races,
first-reference/calendar-correction races, shared spent-credit reversals,
cross-owner constraints, migration failures/version drift and audit update/delete
rejection. SQLite's [transaction semantics](https://www.sqlite.org/lang_transaction.html)
and [deferred foreign-key behavior](https://www.sqlite.org/foreignkeys.html)
support the locking/commit model; the tests exercise the bundled implementation.

Remaining limits: migration identity checks the recorded version/SQL, not a full
fingerprint of live schema objects. The adapter is not a general database forensic
validator, and direct database owners can bypass/drop audit safeguards or rewrite
otherwise consistent state. No application SQL uses audit replacement/upsert;
future code must preserve append-only INSERTs (SQLite REPLACE has special trigger
semantics). Corruption errors require investigation, not automatic repair/retry.
Full disk/IO faults, actual power loss, Linux filesystem behavior and recoverable
backups remain operational validation work. The process-exit test proves recovery
of uncommitted writes, not power-loss durability of acknowledged commits. Existing
M3 identity/request/worker bounds and M6 storage/backup boundaries remain deferred.

Final fresh-review validation:

- `cargo build --workspace --all-targets --locked`: passed.
- `cargo test --workspace --locked --quiet`: 85 test entries passed (including
  the subprocess entry point); both crates' doc tests passed.
- `cargo test --workspace --release --locked --quiet`: the same 85 entries passed.
- `cargo test -p daymark-domain --no-default-features --locked --quiet`: 54 passed.
- `cargo tree -p daymark-domain --no-default-features --locked`: no dependencies.
- `cargo fmt --all --check`: passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- Working and staged `git diff --check`: passed. Existing tests, migration,
  accepted domain files and Cargo manifests/lockfile are unchanged by this review.

Only environmental home-path canonicalization and Git line-ending notices were
emitted. No project compiler or Clippy warning occurred. Milestone 2 is recommended
for human acceptance with the two integrity fixes and documented limits above.
Review changes are committed on `milestone-2-persistence`; no merge to master,
Milestone 3 functionality or real deployment is part of this review.
