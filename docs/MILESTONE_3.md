# Milestone 3: application API, identity and security

## Scope and architecture

Work starts from accepted master `ad16f71` on `milestone-3-api-auth`.
`daymark-api` is a separate Rust/Tokio/Axum library and executable. The domain
and persistence implementations and applied migration remain unchanged. The
review's narrowly justified initializer-test retry correction is described below.
HTTP handlers decode bounded transport values and call the accepted
Store operations; they do not calculate balances, conversions, carryover or
entitlement. There is no frontend, Docker image, deployment or backup tooling.

The versioned same-origin interface is `/api/v1`. JSON imports activate a complete
holiday calendar atomically. The planned frontend can parse CSV into this import
representation and preview it in Milestone 4. No multipart/file storage subsystem
is necessary. Health is available at `GET /health` without authentication.

## Identity, passwords and sessions

Registration accepts only username and password. Usernames are 3–32 ASCII letters,
digits, underscore or hyphen, normalized to lowercase; uniqueness is checked
transactionally. Passwords are 12–128 UTF-8 bytes, with no truncation or composition
rules. Password changes are administrator mediated. No email, self-service reset,
OAuth, first-registrant promotion or default credential exists.

Passwords use RustCrypto Argon2id v19 with independent OS-random salts and the
explicitly tested default profile: 19,456 KiB memory, two iterations, one lane.
At most two hash/verify jobs execute concurrently. Unknown usernames undergo a
dummy verification with the same profile. Unknown-user and wrong-password errors
have the same status and message. Registration conflicts use a generic account
unavailable message, but unique-username registration inherently makes availability
observable. Secrets are never included in logs, account lists or audit events.
Owned password buffers are zeroized after use; this does not claim to erase every
copy made by the HTTP stack or allocator. The implementation follows the
[RustCrypto Argon2 interface](https://docs.rs/argon2/0.5.3/argon2/).

An account has a server-allocated random 64-bit owner identity, stored as the same
eight-byte big-endian value used by the accepted Store. Identity and role cannot
be changed through an update; accounts cannot be deleted. Registration always
creates an ordinary user, even when no administrator exists. Account creation,
settings changes and successful password resets append action/actor/target/UTC
audit records without credential contents.

Sessions use independent 256-bit OS-random opaque tokens. Only SHA-256 token
digests are stored in SQLite. Each session has an independent CSRF secret, a
12-hour absolute expiry, and a 30-minute idle expiry. Login issues a fresh token,
revokes any presented old token, prunes expired sessions and retains at most five
active sessions per account. Logout revokes the current token. Successful reset
changes the password, revokes all target sessions and records the action in one
SQLite transaction. After password verification outside the database worker,
login rechecks the exact verified hash before issuing a session, preventing a
reset/verification race from reactivating an old password.

Production cookies are `__Host-daymark`, `Secure`, `HttpOnly`, `SameSite=Strict`,
`Path=/`, with no Domain and a bounded Max-Age. Every response is `no-store` and
has nosniff, no-referrer and a restrictive CSP; HTTPS mode adds HSTS. Successful
login and `GET /api/v1/session` return the CSRF value. All unsafe methods require
an exact configured Origin, `X-Daymark-Request: 1`, and JSON Content-Type. Protected
mutations also require `X-CSRF-Token`, compared using constant-time digest
comparison. Missing/foreign/null origins and conflicting duplicate security
headers fail closed. Cross-site or same-site Fetch Metadata is rejected. No CORS
permission is emitted. GET/HEAD routes cannot change business/account data;
session activity timestamps can refresh on authenticated reads.

## Authorization and administration

Private endpoints have no owner parameter. The worker authenticates the token
against durable sessions and accounts immediately before performing the protected
operation, then supplies only that principal's owner to Store. Personal records,
effects, settings and history remain scoped to that owner, including for admins.
Unknown/other-owner records receive the same not-found result. History for an
absent owned identity is an empty page, regardless of another owner's IDs.
Unknown fields in request DTOs and filter/page parameters are rejected.

The admin permission gates only complete global holiday replacement and the
basic account list/manual password reset. Admin status does not grant an owner
override or a private-ledger route. Referenced-holiday errors contain no owner,
source ID or notes. Account lists include only ID, username, role and creation
time. Reset targets must be ordinary accounts; no admin-password reset endpoint,
role editor or account deletion feature is introduced.

Initial bootstrap is a local interactive operator command, run while the server
is stopped. It prompts twice with terminal echo disabled, refuses nonterminal
input, creates a new admin account, and fails if the username is taken or an
administrator already exists. It never promotes an existing registrant. The
initial admin password must be retained by the operator; loss of that credential
does not enable an unauthenticated recovery feature.

```text
cargo run -p daymark-api --locked -- bootstrap PATH_TO_LOCAL_DATABASE USERNAME
cargo run -p daymark-api --locked -- serve PATH_TO_LOCAL_DATABASE https://daymark.test 127.0.0.1:3000
```

The application speaks HTTP to the eventual HTTPS ingress; the configured origin
must be the external origin. These are local invocation examples, not deployment
scripts or authorization to deploy. For explicit loopback development only:

```text
cargo run -p daymark-api --locked -- serve PATH_TO_LOCAL_DATABASE http://127.0.0.1:3000 127.0.0.1:3000 --local
```

Local mode requires a loopback origin and bind, and uses a separate `daymark-local`
HttpOnly/SameSite cookie without Secure. HTTPS configuration cannot silently fall
back to this mode. Production cookie behavior is exercised by HTTP tests even
though test traffic itself is local and has no TLS terminator.

## Storage and bounded blocking execution

Accounts, settings, sessions, account audit and auth throttles share the SQLite
file with the accepted ledger. The application has an independent, exact-text
checked `identity_migrations` history. It does not alter the applied M2 migration
or Store's migration checker. Failed identity initialization rolls back. A clean
M2 schema can be upgraded; a database containing unclaimed M2 source owners is
rejected rather than silently binding them to registered accounts. M2 contained
no real application accounts or legacy-data import requirement. Additional
triggers require an existing account for source insertion and prevent owner
changes, supplementing the authenticated application boundary and M2 composite
ownership keys. The application does not expose arbitrary SQL.

One application database worker serializes session validation, personal operations,
logout and reset. A semaphore is acquired before `spawn_blocking`; at most one
database closure runs and at most 32 database operations are admitted. A separate
two-permit semaphore bounds password hashing. Permits move into the blocking
closure, so cancellation cannot release capacity while a job continues running.
SQLite and password work never execute on Tokio's async executor threads. The
executable uses two runtime workers and a maximum of four blocking threads.

An exclusive OS lock on a sibling `.application.lock` file enforces **one
application process per database**, including bootstrap. Existing database paths
are canonicalized before locking, dangling database symlinks are rejected before
SQLite can create their target, and the lock suffix is appended to the full
filename. The lock is held for
the lifetime of the database worker and released by the OS on exit. Never delete
or replace a live lock file, or open one database under hard-link aliases.
Use a single operator-owned canonical database path and private directory. The
application is not designed for multiple server replicas. This restriction keeps
authentication and subsequent Store operations in the same serial ordering as
logout/reset without changing the accepted Store transaction API. Filesystem
operators and code that bypasses the application remain trusted, as in M2.

There are two SQLite connections inside that worker: the accepted Store and the
identity adapter. Both enable foreign keys and FULL durability; Store establishes
WAL. Identity mutations are parameterized immediate transactions. Store continues
to acquire its own immediate transaction before loading and validating source
mutations. Source/effect/audit writes remain atomic. Busy, storage, integrity or
encoding failures return generic 503 errors without SQL or private contents.
There is no automatic mutation retry and no cached validated candidate. A client
retry invokes the entire authenticated operation and fresh Store validation.
After a connection loss clients must reload to determine the accepted state;
aborting a request does not undo a transaction already running.

## Resource bounds and abuse resistance

At most 64 requests pass the application admission gate concurrently. Bodies are
bounded to 32 KiB even without Content-Length, with a ten-second read deadline;
URIs are bounded to 4 KiB and decoded headers to 16 KiB. Notes allow 8 KiB, included
date lists at most 366 values, holiday names 200 bytes, notes filters 512 bytes,
and result pages 1–100 items with bounded offsets. Domain constructors still
validate dates, whole-hour values and all business invariants. Transport rejection
does not introduce a second balance policy. Axum integration follows its
[middleware interface](https://docs.rs/axum/0.8.9/axum/middleware/fn.from_fn.html).

Durable fixed-window throttles run before hashing and count successful as well
as failed attempts: 120 combined auth attempts per ten minutes globally, 20 login
attempts per ten minutes per socket-peer address, ten per normalized username,
and five registrations per hour per socket-peer address. Limits return 429 with
Retry-After. A throttle table admits at most 4,096 keys; exhaustion rejects new
keys rather than evicting another client's live protection. Expired entries are
pruned. Address/username limiter keys are hashed and contain no passwords.
State survives application restart. A shared global limit also bounds distributed
hashing abuse. No forwarded address headers are trusted in M3. Behind a proxy,
the address limit therefore applies to the proxy's combined traffic; explicit
trusted-proxy configuration and ingress connection/header timeout limits require
operator integration before internet exposure in later milestones.

The accepted adapter still loads the current owner aggregate and writes complete
affected-year audit snapshots. Ledger pages limit transport output, not internal
aggregate projection work. History is SQL-paginated before decoding rather than
loading all revisions. Individual Change/snapshot payloads can grow with a large
ledger. Worker bounds prevent uncontrolled concurrent execution, but do not prove
production capacity, bound total disk growth or replace a release load test.

## JSON API contract

All private requests send the session cookie. All mutations send Origin,
X-Daymark-Request, JSON Content-Type and, after login, X-CSRF-Token. Dates are
`YYYY-MM-DD`. IDs, source/calendar revisions, hours, signed deltas and returned
balances are decimal strings to preserve the entire accepted integer range in
JavaScript. Years and UTC audit timestamps are JSON integers. Null settings and
calendar revisions are supported; first calendar configuration supplies null.
Enum bucket values are `Pto`, `Comp`, `Holiday`, `Floater`; classification values
are `Grant`, `Use`, `Accrual`, `Conversion`, `Adjustment`.

| Method/path under `/api/v1` | Request / result |
| --- | --- |
| POST `/register` | `{username,password}`; 201 basic ordinary account, then login |
| POST `/login` | `{username,password}`; 200 account/CSRF and new cookie |
| POST `/logout` | 204, current session revoked and cookie cleared |
| GET `/session` | Current account and CSRF; 401 for revoked/expired token |
| GET / PUT `/settings` | Informational `{hire_date,annual_pto_allowance}`; no grants |
| GET `/years/{year}/snapshot` | Annual balances, configured flag and linked effective effects |
| GET `/years/{year}/ledger` | Effective effects, newest first; bucket/classification/from/through/notes/offset/limit filters |
| POST `/sources` | `{id,source}`; 201 accepted Change |
| GET `/years/{year}/sources/{id}` | Current owned Record |
| PUT `/years/{year}/sources/{id}` | `{revision,source}`; accepted Change; path year is expected old year |
| DELETE `/years/{year}/sources/{id}` | `{revision}`; accepted deletion Change |
| GET `/sources/{id}/history` | Owned Change page, including deletion evidence and deliberate year moves; offset/limit |
| GET / PUT `/years/{year}/holidays` | Read global versioned calendar; admin PUT `{revision,holidays:[{id,date,name}]}` |
| GET `/admin/accounts` | Admin-only basic account page; offset/limit |
| POST `/admin/accounts/{id}/password` | Admin-only `{password}` for an ordinary target; 204 |

`source` contains `{dates,activity,notes}`. Input activity uses `kind`:

```json
{"id":"1","source":{"dates":["2026-01-01"],"activity":{"kind":"grant","bucket":"Pto","hours":"160"},"notes":"Explicit annual opening amount."}}
```

Kinds: `grant`/`use` with bucket/hours, `adjustment` with bucket/delta,
`comp_work` with work, `holiday_use` with holiday ID, and `holiday_work` with
holiday ID/work. Work contains nullable hours_worked and multiplier plus required
credited_comp. Multiplier is null, `OneToOne` or `OneAndAHalf`; never a calculator.

Read Record and Change shapes explicitly expose the accepted source/effect
relationships: `reference:{id,revision}`, `source`, and Change's before/after,
actor/time and before_effects/after_effects snapshots. Read activity/origin use
the externally tagged variant names (`Grant`, `CompWork`, `HolidayWork`, `Source`,
`WorkedHoliday`, etc.); the frontend maps these to the input `kind` forms. These
are version-one transport choices, distinct from the internal stored encoding:
the wire boundary converts dates and full-width numbers, and request DTOs reject
unknown fields. Notes remain JSON text; the future frontend must insert them as
text, never trusted HTML.

Errors use `{error,message}`. Domain errors are 422 with useful owned bucket/year
context; source or calendar staleness is 409 with `stale_revision` or
`stale_calendar`; missing records 404; unauthenticated 401; forbidden 403;
malformed/bounds 400 or 413; throttling 429; storage/admission failure 503.
Malformed path/method rejections may use Axum's non-JSON client-error format.
An expected year that no longer matches a moved record follows the accepted
Store contract and returns 404, rather than guessing the record's new year.

## Acceptance evidence and review

HTTP tests send untrusted requests through the real Axum router, cookies,
middleware, extractors, handlers and SQLite. A real loopback TCP listener test
also checks registration/login/cookies and cross-origin rejection. Synthetic
clock and database fault injection make expiry and rollback assertions
deterministic; no credential or data from a real user is used.

New boundary proof covers AUTH-001–009, AUDIT-003, ADMIN-001–004, owner-bound
LEDGER-001–006 and existing audit/atomicity/stale-revision behavior. UI acceptance
and operational DB/container/backup criteria remain deferred. Tests cover:

- Colliding full-width IDs, guessed IDs, owner injection, notes search, paged audit
  after deletion, admin private-data denial, and unchanged second-owner records.
- Salted Argon2 hashes, username normalization, dummy-login error equality,
  cookie attributes, fixation/rotation/reuse, idle/absolute expiry, five-session
  cap, logout/reset revocation, duplicate cookies and CSRF/origin attacks.
- No first-user promotion, secure bootstrap conflicts, admin account lists,
  denied ordinary-user resets/calendar writes, and referenced holiday correction.
- Concurrent overspends and stale edits, invalid/stale calendars, malformed or
  oversized bodies, whole-hour/date bounds and literal SQL/HTML-like notes.
- SQLite source-audit and reset-audit failures, safe busy handling and fresh
  retries; informational settings, durable session/data/settings reopen, migration
  drift, rejected unclaimed ownership and enforced single-instance startup.
- Durable login/registration throttles, ignored spoofed forwarding headers,
  limiter cardinality, bounded DB admission and cancellation-safe hash permits.

An independent reviewer read the authoritative requirements and inspected the
implementation and tests. The reviewer added three HTTP tests in
`application/tests/independent_review.rs`: swapped CSRF identities and duplicate
security headers; registration/login-rotation transaction rollback; overlapping
old-password login and admin reset. No authentication, admin or cross-owner
bypass was reproduced. The overlap test observes a scheduler-chosen ordering;
supplemental unit tests now force reset specifically between credential read and
session issuance, and expiry/revocation specifically after queue admission but
before protected execution. Both reject stale authority without executing the
protected operation.

The reviewer identified lock-path aliases as a single-instance risk. Remediation
canonicalizes existing database files, rejects dangling database symlinks using
`symlink_metadata` before any lock/database creation, and appends the lock suffix
to the complete filename so distinct extensions cannot collide. Tests exercise
repeated canonical startup and distinct filenames; an additional Unix-only test
exercises existing/dangling symlinks and awaits Linux execution. Hard-link aliases
and concurrent filesystem manipulation remain outside the trusted operator
contract. The reviewer checked these fixes and confirmed no outstanding
actionable findings.

The review's first full debug run exposed an existing M2 test flaw: all four
initializers begin simultaneously, but the test allowed only one immediate retry
when WAL activation returned SQLITE_BUSY. A second full open was observed
returning BUSY while other initializers still contended. The only accepted-test
change is in `persistence/tests/independent_review.rs`: retry the **complete open**
only for SQLITE_BUSY, with a 15-second deadline and ten-millisecond backoff. Every
other error still fails immediately, and all four concurrent initializers plus
the original schema and integrity assertions remain. This corrects a demonstrated
flaky timing assumption; it does not weaken a business rule, ignore an error,
change Store behavior or retry a previously validated ledger candidate. The
reviewer reran the targeted case and all 31 persistence test entries successfully.

No other accepted M1/M2 test, domain implementation, persistence implementation
or applied migration changed. No unresolved product/domain ambiguity was found.

Final validation on Windows with Rust/Cargo 1.97.1 and bundled SQLite:

- `cargo build --workspace --all-targets --locked`: passed.
- `cargo build --workspace --release --all-targets --locked`: passed.
- `cargo test --workspace --locked --quiet`: 110 test entries passed.
- `cargo test --workspace --release --locked --quiet`: the same 110 passed.
- The totals include 25 application entries: four deterministic resource/race
  tests, three independent HTTP review tests, six runtime/migration/fault tests,
  and twelve HTTP security/isolation tests. The remaining 85 are the accepted
  domain/restore/persistence suite, including its subprocess test entry point.
- `cargo test -p daymark-domain --no-default-features --locked --quiet`: 54 passed.
- `cargo tree -p daymark-domain --no-default-features --locked`: no dependencies.
- All workspace doc tests passed (no examples).
- `cargo fmt --all --check`: passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- Working and staged `git diff --check`: passed before commit.

The earlier review run's initializer flake and its correction are disclosed above;
the complete final debug and release suites passed after remediation. The
Unix-only symlink regression is not counted as executed on this Windows host.
Environmental home-path canonicalization and Git line-ending notices occurred;
no project compiler/Clippy warnings occurred. Work ends at Milestone 3 on
`milestone-3-api-auth`; master is not merged or modified by this milestone.

## Deferred boundaries

Milestone 4 owns frontend forms/rendering, CSV preview, responsive design and
browser behavior. M5 owns browser acceptance. M6 owns Linux/container/TLS ingress,
connection limits, trusted proxy policy, private storage permissions, backup and
restore, and operator packaging. M7 owns hostile release and capacity review.
This milestone proves neither Linux filesystem lock behavior nor backup/power-loss
recovery. The single application process is a deliberate supported configuration;
horizontal replicas require a redesigned shared authorization/transaction boundary.
No real deployment or merge to master is part of this work.

## Fresh independent release review (2026-09-06)

Baseline: `0f3aeed` on `milestone-3-api-auth`. This review read AGENTS.md,
CODEX_RUNBOOK.md, every document under docs, and the complete domain, persistence,
application, migrations and test implementations. It did not rely on the previous
review's conclusions. Work remains confined to Milestone 3.

One defect was reproduced through a real loopback Axum/hyper TCP listener. Login
used `session(...).ok()`, treating malformed or ambiguous session headers as if no
session had been presented. A request with duplicate session cookies returned 200
and issued a new cookie; the error path skipped revocation of the presented old
session. Duplicate CSRF headers took the same path. Correct username/password and
the normal Origin/custom-header checks were still required: this was a
fail-closed parsing and session-rotation defect, not a password or owner bypass.

Login now distinguishes an absent session from a parsing error before database
admission/password work. Ambiguous cookie/CSRF headers are rejected without
issuing or revoking sessions. Missing cookies and syntactically valid expired or
unknown cookies still permit normal password authentication. Protected routes use
the same parser and still require a session. No migration, dependency, domain or
persistence production code changed.

Eight tests were added; no existing test or assertion was modified or removed:

- A TCP login regression covers repeated identical/different session cookies,
  malformed duplicate cookies, identical/conflicting CSRF headers, unchanged
  existing sessions, and login with an expired cookie. It failed with HTTP 200
  before the fix and passed after it.
- Raw TCP mutations exercise missing/duplicate/foreign/null/misleading Origin,
  non-ASCII and Fetch Metadata headers, content types, swapped CSRF credentials,
  cookie ambiguity, conflicting/invalid Content-Length, duplicate chunked
  encoding, unsupported methods and a chunked body exceeding 32 KiB. Rejections
  leave sources and source audit empty.
- Chunked TCP creates with substituted Cookie/CSRF trailers retain the initial
  authenticated owner. Concurrent TCP spends have exactly one winner in each of
  PTO, Comp and Floater, with exact durable source/audit counts and no second-owner
  data or balance change.
- Sixty-four incomplete TCP bodies use Expect/Continue handshakes to prove actual
  request admission. Request 65 receives 503; completing bodies releases capacity.
  An unfinished body receives 408 after the read deadline and capacity recovers.
- HTTP year moves preserve owned audit, reject stale years/revisions, and reject
  injected effect corruption on reads and writes with generic 503 responses.
  Rejected writes preserve audit and the other owner's data.
- Concurrent normalized-username registration produces one account, Unicode/NUL/
  whitespace lookalikes fail validation, 128-byte UTF-8 passwords round-trip
  without truncation, overlong passwords fail, and raw duplicate JSON keys fail.
- Separate executable processes reject live locks for normal, dot-component and
  canonical database paths on this Windows host.
- A deterministic worker test holds an authenticated mutation inside blocking
  execution, fills all 32 database admissions, aborts its caller, cancels queued
  readers, and queues logout. The running job retains its permits/lock, commits
  once under the original owner, then logout revokes subsequent authority. This
  uses test-only synchronization rather than claiming a chosen TCP-disconnect
  schedule. The prior cancellation-safe password-worker tests remain in force.

The new HTTP tests are in `application/tests/release_review.rs`; the deterministic
worker test is in the existing test-only module in `application/src/lib.rs`.
Previously accepted security tests still cover reset/login ordering, expiry and
revocation after queue admission, ordinary/admin route boundaries, cross-user
read/edit/delete/search/history, session fixation/caps, durable throttles/restart,
SQLite busy failures, and registration/reset/session transaction rollback.

No cross-user data access, administrator private-data route, ordinary-user admin
bypass, stale-authority execution or negative-balance acceptance was reproduced.
An administrator able to reset a password remains a trusted account-support actor;
route isolation does not prevent that actor from subsequently logging in with the
password they set. This follows the required manual-reset model.

Transport observations and deferred risks: hyper can reject malformed framing
before application middleware, so its parser-generated errors do not carry the
middleware's JSON envelope/security headers; those responses contain no account
or ledger data. Application admission starts after request headers are parsed.
Pre-header connection limits/timeouts and slow response readers still need the
planned ingress/capacity review. Bounded worker concurrency does not bound total
ledger/audit size, aggregate projection cost, response size or disk growth. Shared
proxy/global throttles can deny service to legitimate users; forwarding headers
remain untrusted. Hard-link aliases and replacement of a live lock file remain
outside the trusted filesystem-operator contract. Linux symlink/lock behavior,
full disk/IO/power-loss tests, TLS/proxy integration, backups, Docker and browser
acceptance remain deferred to their documented milestones. None was implemented
or deployed by this review.

Final fresh-review validation on Windows, Rust/Cargo 1.97.1 and bundled SQLite:

- `cargo build --workspace --all-targets --locked`: passed.
- `cargo build --workspace --release --all-targets --locked`: passed.
- `cargo test --workspace --locked --quiet`: 118 test entries passed.
- `cargo test --workspace --release --locked --quiet`: the same 118 passed.
  These include all 33 API/security/worker entries and the 85 accepted
  domain/persistence entries, including the persistence subprocess entry point.
- `cargo test -p daymark-domain --no-default-features --locked --quiet`:
  54 passed; `cargo tree` with the same package/features confirms no dependencies.
- All workspace doc tests passed (no examples).
- `cargo fmt --all --check`: passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- Working and staged `git diff --check`: passed before commit.

The deliberately failing pre-fix TCP regression is disclosed above; complete
debug/release suites passed after remediation. Only environmental home-path
canonicalization and Git line-ending notices occurred. Unix-only tests were not
executed on this host. No accepted tests, applied migrations, dependencies or
M1/M2 implementation changed. Milestone 3 is recommended for human acceptance
with the parsing/rotation fix and the stated operational limits. Review changes
are committed on `milestone-3-api-auth`; master is unchanged, and work stops here.
