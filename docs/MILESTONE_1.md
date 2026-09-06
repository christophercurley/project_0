# Milestone 1: pure domain model

## Implemented design

The `daymark-domain` library uses only the Rust standard library. `model.rs`
defines validated date/year/hour values, explicit source activities, calendar
configuration, effects, filters and revision/audit values. `ledger.rs` implements
deterministic candidate-state validation, effective projections, user-scoped
queries/mutations and retained in-memory history. There is no infrastructure code.

The caller supplies authenticated ownership and UTC timestamps; authorization and
clock acquisition are intentionally outside the domain. Calendar configuration
is an explicitly privileged application boundary. No administrator permission
implicitly changes the personal-record access model.

Amounts and source hours worked are whole hours. Use/grant zero amounts and zero
adjustments are rejected as empty ledger operations; zero confirmed Comp is valid
for holiday work that only converts entitlement. Hour parsing rejects fractional,
negative, exponential and out-of-range inputs. Checked wide summation allows valid
net balances independent of insertion order without overflow. Display days use
exact eighth-day fractions, not floating point.

Source dates are explicit, sorted, unique and contained in one year. Multi-day use
is one total effect, with no weekday, daily-hour or overlap inference. Other
activities have a single date. Date filtering matches actual included dates, not
gaps in their enclosing range; ordering uses the earliest included date descending.
These are presentation/query conventions, not daily balance allocations.

The annual snapshot includes every current record regardless of date relative to
today. Year-changing edits validate both affected years. No automatic year grants,
carryover, caps or expiry exist. Holiday entitlement is ten global eight-hour
effects and cannot be manually granted or adjusted. Holiday use and work validate
the selected stable holiday ID against its configured date.

Conversion effects are keyed by holiday within an owned annual projection and
link all supporting source IDs/revisions. Additional work adds only independent
confirmed Comp. Edits recompute both old and new holiday state; deletion removes
conversion only with the last support. Failed changes, including failures caused
by previously spent Comp/Floater, preserve all current state and audit history.

IDs are scoped to users; holiday IDs are scoped to years. Deleted source IDs cannot
be reused. Edits/deletions require the expected current revision. Audit changes
retain source before/after values, UTC timestamps and complete affected-year
before/after projections. Global calendar revisions preserve historical names and
dates. Active references block date changes/identity replacement; deliberate
source correction or deletion releases them while retaining audit evidence.

## Acceptance traceability

Tests live in `tests/domain.rs` and `tests/review.rs` (independent review).
Coverage here means pure-domain proof, not completed HTTP or browser acceptance.

| Acceptance IDs | Domain evidence | Remaining product proof |
| --- | --- | --- |
| UNIT-001, UNIT-002 | Strict integer durations, exact days including large values | UI rendering |
| BAL-001 through BAL-005 | All-bucket rejection; full-state rollback; overflow and multi-effect failures | SQLite atomicity and API errors |
| PTO-001 through PTO-004 | Explicit grants/use, no carryover, isolated years | User entry/browsing UI |
| HOL-001 through HOL-007 | Ten valid global entitlements, exact use, dates, duplicate prevention, atomic invalid configuration | CSV parsing/upload and admin authorization |
| FLT-001 through FLT-004 | Partial work, one shared conversion, multiple supports, Floater use | UI representation |
| COMP-001 through COMP-005 | Confirmed amounts, no inference, integer metadata, retained notes | Form/display flows |
| COMBO-001 through COMBO-003 | Linked effects/support revisions, edit/delete reconciliation and history | Visible relationships and durable transactions |
| MULTI-001, MULTI-003 | One deduction, edit/delete coherence | Product forms |
| MULTI-002, CAL-001, CAL-002 | Explicit included dates, global dates, bucket/effect identities supplied | Calendar visual acceptance deferred |
| LEDGER-001 through LEDGER-006 | Effective ordering, precise bucket/classification/date/notes filtering and ownership | API/UI filters |
| AUTH-003 through AUTH-005 | Owned domain lookups/mutations/search cannot operate on another owner's data | Authenticated identity binding and direct HTTP attack tests deferred |
| AUDIT-001 through AUDIT-004 | Before/after state, tombstones, owned history and effective-only sums | Durable history and UI access |
| YEAR-001 through YEAR-004 | Valid calendar dates, independent years, no implicit rollover, explicit opening entries | Historical browsing UI |
| ADMIN-002, ADMIN-003 | Complete calendar validation, safe referenced corrections | Admin role and import/correction interface |
| DEPLOY-002, BRAND-001 | No deployment; neutral synthetic names/fixtures | Continued repository/release review; no prohibited identity embedded in a scanner |

DASH, UI, LEDGER-007, full authentication, ADMIN-001/004, DB and remaining DEPLOY
criteria await their planned milestones. Settings are also deferred; the domain
does not use hire date or allowance to infer grants.

## Test strategy and limitations

Targeted tests cover approved owner decisions and boundary/adversarial behavior.
An independent boolean-state oracle enumerates 1,296 four-operation sequences
(5,184 transitions) of support creation/deletion, Holiday Use and Floater use.
It checks accepted/rejected operations, rollback, conversion cardinality, balances
and unaffected second-user state. This is deterministic exhaustive bounded testing,
not a claim to cover every possible sequence.

Run the build, complete tests, formatting and denied-warning Clippy commands in
README.md. Preserve existing valid tests during remediation.

Independent review found no justified domain-rule defects or blocking ambiguity.
It added four adversarial tests for cross-year debit rollback, overflow caused by
deleting a debit, shared conversion supports split across years with audit links,
and date correction blocked until every user's active references are resolved.
No existing tests were weakened or changed to obtain a pass. The full suite now
contains 49 tests, including the bounded sequence oracle.

Final validation on Rust/Cargo 1.97.1:

- `cargo build --all-targets --locked`: passed.
- `cargo test --locked`: 49 passed; doc tests passed (no examples).
- `cargo test --release --locked`: 49 passed.
- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --locked -- -D warnings`: passed.
- `git diff --check`: passed.

The host toolchain emitted a home-path canonicalization warning; commands exited
successfully and project compilation/Clippy produced no warnings.

Remaining engineering risks are intentionally deferred: authenticating supplied
IDs, durable audit, concurrent database transactions, import transport, browser
rendering and operational security. The reference model clones in-memory state
and records complete affected-year snapshots for clarity; it is not a storage or
performance prescription for the later SQLite adapter. Dates support years 1–9999;
hour amounts fit signed 64-bit integers. Out-of-range values are rejected.

No unresolved blocking product ambiguity remains. Milestone 2 has not begun.

## Fresh independent release review (2026-09-06)

Reviewed the full domain implementation and existing tests against all four
authoritative documents, the approved decisions, AGENTS.md and CODEX_RUNBOOK.md.
No reproducible Milestone 1 correctness defect or blocking ambiguity was found.
No production code or existing test was changed.

Three additional adversarial tests in `tests/review.rs` cover:

- Exhaustive replacement/deletion of either of two sources across seven activity
  cases, two holidays, and combinations of spent Comp/Floater. An independent
  state-table oracle determines validity and balances. Assertions also check
  whole-ledger rollback, preserved prior audit, exact before/after projections,
  shared support revision links, stale edit/delete rejection, and a second user
  with colliding source IDs.
- Signed-boundary adjustment edits for PTO, Comp and Floater, including a valid
  `i64::MIN` debit whose annual net is zero, and overflow rejection with rollback.
- Identical effective projections across all six insertion orders of three
  holiday-work records, including shared support and a second holiday.

The exhaustive test is bounded to the stated cases; it is not a proof over all
possible inputs or histories. Existing tests additionally exercise future-dated
net balances, year-changing edits, multi-day included-date filtering, holiday
calendar correction locks, and shared-support deletion with spent credits.

Validation after the additions: `cargo build --all-targets --locked`,
`cargo test --locked` (52 passed), `cargo test --release --locked` (52 passed),
`cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, and
`git diff --check` all passed. Doc tests passed with no examples. Host path
canonicalization and Git line-ending notices remain environmental; no project
compiler or Clippy warning occurred. README now includes release tests and diff
checks in the validation commands.

Human acceptance of Milestone 1 is recommended. The in-memory model still relies
on trusted caller identity/time, and cloning the full ledger and audit snapshots
does not scale as a production storage strategy. Durable atomicity, authenticated
ownership, concurrency and request bounds require later-milestone proof. In
particular, calendar configuration currently replaces a complete calendar without
an expected-revision argument; the later administration/persistence boundary must
address stale concurrent calendar submissions. This does not violate the current
pure-domain milestone's sequential configuration contract. Source edits/deletions
already enforce expected revisions. No Milestone 2 work or merge is authorized
by this recommendation.
