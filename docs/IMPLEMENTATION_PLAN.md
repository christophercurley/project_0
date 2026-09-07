# Daymark implementation plan

Status: planning gate approved by the product owner. Daymark / DMK is provisionally approved. No repository rename, DNS change, or production deployment is authorized.

## Authority and approved decisions

Follow AGENTS.md and DOMAIN.md > ACCEPTANCE.md > ARCHITECTURE.md > PRODUCT.md. The following explicit product-owner clarifications resolve the planning questions:

1. Balance means the selected calendar year's net effective balance, never a chronological running balance. Future-dated effective entries count immediately for display and validation.
2. Referenced holiday names may change. Dates may change only when no active user records reference the holiday; affected records must first be deliberately resolved. Never silently migrate historical activity. Preserve previous configuration in audit history.
3. Conversion is state-based per user/holiday, not owned by an arbitrary event. At least one active worked-holiday source means exactly one -8 Holiday/+8 Floater conversion. Additional sources may independently credit confirmed Comp. Deleting one of several sources preserves conversion; deleting the last reverses it atomically, subject to nonnegative annual balances. Holiday Use must be explicitly corrected before recording the holiday as worked.
4. No arbitrary Holiday Grant/Adjustment. Its entitlement derives exclusively from the ten configured holidays and is consumed only by Holiday Use or Holiday-to-Floater conversion. PTO, Comp, Floater support explicit grants/adjustments.
5. All durations, including optional hours worked, are whole hours. No fractional duration input.

## Complete product architecture

One Rust/Tokio/Axum application serves a same-origin JSON API and static HTML/CSS/vanilla JavaScript. SQLite lives on persistent mounted storage. Separate pure domain rules from application transaction coordination and HTTP/persistence adapters. Start with a small domain crate; introduce an application crate in later milestones. No frontend/CSS framework, microservices, queues, Redis, generalized policy engine, or complex event sourcing.

Exactly four buckets: PTO, Comp, Holiday, Floater. Use checked integer hours and eight hours per display day. Stable user-owned source events generate explainable effects. Represent grants, signed adjustments, uses, Comp work, Holiday Use and worked holidays explicitly. Retain optional whole hours worked, multiplier context, confirmed Comp and notes. Never infer credit from work date, duration or multiplier. Derive shared holiday conversion from all active supporting source events, linking every support revision to the conversion.

Multi-day use stores one total and explicit included dates, deducting once. All dates belong to one calendar year; cross-year vacations require separately entered yearly totals. No inferred weekend exclusions, daily caps, or overlap prohibition. No automatic PTO entitlement, grants, carryover enforcement/transfers, Comp expiry/cap, or annual processing. New-year opening entries are explicit. Hire date and annual PTO allowance are informational settings only.

Validate a candidate complete effective state before applying any mutation. Reject the whole mutation when any bucket/year would become negative. Audit revisions never count as effective entries. Unconfigured holiday years expose configuration absence and disallow Holiday use/work; other buckets remain usable.

## SQLite persistence (Milestone 2 onward)

Use explicit SQL, versioned migrations, foreign keys, WAL, bounded connections and busy timeout; SQLx is a reasonable initial choice. Tables cover accounts/settings, hashed-token sessions, versioned annual calendars and stable holiday IDs, owned source/current revisions and included dates, effective ledger effects, unique user/holiday consumption, shared conversion support links, and immutable audit revisions.

Acquire the write transaction before reading validation state; commit sources, effects and audit together. Check expected revisions to reject stale edits. Scope every personal query/search/history/mutation to authenticated ownership and use composite ownership constraints. Do not cache balances initially. Index owner/year/date and source-effect relationships. Project global Holiday entitlement without copying a calendar for each registering user. Reject referenced holiday date changes or identity removal; permit audited name corrections. Validate/activate all ten holidays atomically.

## Authentication and administration (Milestone 3 onward)

Open unique-username/password registration, login/logout and manual administrator reset only. No email, user-facing recovery, OAuth, SSO, 2FA or CAPTCHA without demonstrated need. Use Argon2id with salts and bounded hashing concurrency; random opaque server-managed sessions with hashed tokens in SQLite; Secure/HttpOnly/SameSite HTTPS cookies; expiry, login rotation, logout/reset revocation; CSRF and origin checks; bounded account/address throttling with explicit trusted proxies; parameterized SQL and request bounds.

One administrator permission category: global holidays, basic account support and manual password reset. It grants no private ledger/audit access. Bootstrap via local operator command with interactive secret input, no defaults or first-registrant promotion. No account deletion feature in v1.

M3.1 product-owner clarification: exactly one administrator account is supported.
Loss of its password is recoverable through a local interactive operator command
holding the same exclusive canonical database lock. It selects the existing sole
admin, requires hidden double input under the same password/hash policy, and
atomically replaces the password, revokes all admin sessions and appends a
secret-free operator audit event. No account selector, promotion, new admin or
remote recovery is permitted. Ordinary users and their data remain unchanged.

## Audit design

Immutable source revisions plus current state, not full event sourcing. Capture actor/owner, UTC timestamp, before/after source values, deleted tombstones, and before/after generated effects with support revisions for shared conversions. Record successful configuration/account changes without credential contents. Write history atomically with effective state. Default Ledger excludes deleted/superseded records; authorized details reconstruct revisions. Failed domain mutations leave state and accepted-change history unchanged.

## Frontend (Milestone 4 onward)

First-party CSS and vanilla JS modules. Dashboard: four hours/day cards with Ledger drilldown and upcoming holidays. Ledger: newest first, year/bucket/classification/date/notes filters, add/edit/delete and unobtrusive history, understandable shared effects. Calendar: month/year views, holidays, bucket labels and coherent multi-day spans. Supporting registration/login, settings and narrow admin screens. Specialized source forms distinguish confirmed Comp from multiplier context. CSV date/name holiday import with complete preview and atomic activation; correction preserves stable IDs. Mobile-first, polished desktop, dark mode, semantic controls, keyboard/focus/contrast and reduced-motion support. No charts, exports or notifications.

## Validation strategy

Map acceptance IDs to automated tests or explicit visual checks. Pure deterministic domain tests first, including adversarial/invariant sequences, integer overflow, year isolation, nonnegative net totals, shared conversion support, atomic rollback, edits/deletes and reconstructable audit. Later add temporary SQLite migration/rollback/concurrency tests; HTTP authentication/authorization/CSRF/session/throttling tests; cross-user guessed-ID/search/history tests; browser critical flows and responsive/dark-mode checks; container replacement and backup/restore tests. Use neutral synthetic fixtures. Never commit real secrets/data or the prohibited identity, including in a scanner; any private identity scan must not print it.

Every completed implementation milestone builds, passes all available tests, cargo fmt --check and Clippy with project warnings denied. Independent review follows implementation, then remediation and complete validation before human review. Never weaken valid tests to pass.

## Backup and deployment (Milestone 6 onward)

Multi-stage Docker build, non-root runtime, persistent database directory including SQLite sidecars. Provide safe placeholder configuration, health checks, structured logs excluding credentials/tokens/hashes/notes, and operator initialization/start/update/backup/restore scripts. Consistent SQLite online backups with integrity checks go to persistent storage. Provide host scheduler examples for daily backups and documented retention (initial default fourteen daily snapshots), plus verified-copy workflow elsewhere on the VPS. Demonstrate isolated restore including accounts, sources, effects and audit. Document stopped writes for restore, preserving previous DB, WAL handling and validation. Document HTTPS ingress using operator host setup. No real VPS access, DNS changes, production users/database/container operations or installed production schedules by agents.

## Milestone sequence

1. Pure Rust domain and automated harness only. No Axum/Tokio server, SQLite, authentication, frontend, Docker or deployment tooling.
2. SQLite persistence, migrations, atomic audit and concurrency tests.
3. Axum API, identity, sessions, admin and security/isolation tests.
4. Static Dashboard/Ledger/Calendar and supporting forms.
5. Full browser acceptance, accessibility and visual refinement.
6. Local Docker and operator artifacts; proven backups/restores/container replacement.
7. Adversarial release/security/performance review, remediation and human handoff.

Follow implementation, independent review, remediation and human review at boundaries. Do not begin the next milestone without authorization.

## Risks and remaining decisions

Main risks: shared conversion reconciliation when supports move/delete; global corrections interacting with private historical activity; concurrent overspending; audit/effect drift; resource abuse on a small server; and backups that have not been restore-tested. Address with whole-state candidate validation, stable holiday identities, transactional writes, tests and local restore proof.

No blocking product ambiguity remains after the approved decisions. Date-only business dates, UTC audit timestamps, informational settings, explicit multi-day included dates/year splits, unconfigured-year handling, atomic ten-holiday activation and no extra v1 features are approved assumptions. Username limits/normalization, session lifetimes, input bounds and retention are documented engineering defaults. No automatic production actions are authorized.
