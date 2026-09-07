# ACCEPTANCE.md

## Purpose

These acceptance criteria define externally meaningful proof of correctness.

Agents should convert these into automated tests where practical.

Passing existing tests is not sufficient if these behaviors are not covered.

## A. Units and balances

### UNIT-001 — Whole-hour accounting
Authoritative bucket quantities are whole hours, never floating point.

### UNIT-002 — Day display
A 16-hour bucket balance is shown as 16 hours and 2 days.

### BAL-001 — No negative PTO
With 4 PTO hours, attempting to use 8 is rejected and balance remains 4.

### BAL-002 — No negative Comp
With 0 Comp hours, attempting to use Comp is rejected atomically.

### BAL-003 — No negative Holiday
An operation cannot consume more Holiday entitlement than is available.

### BAL-004 — No negative Floater
An operation cannot reduce Floater below zero.

### BAL-005 — Failed multi-effect operation is atomic
If one required effect of a multi-effect operation fails, none become active.

## B. PTO

### PTO-001 — Explicit annual grant
No PTO grant appears silently. Recording a 160-hour PTO Grant increases PTO by exactly 160.

### PTO-002 — PTO use
160 PTO minus an 8-hour Use equals 152 PTO.

### PTO-003 — No automatic carryover
A prior-year ending PTO balance is not automatically transferred to the next year.

### PTO-004 — Prior year preserved
Browsing a prior year shows that year's ledger/balances without current-year mixing.

## C. Holidays

### HOL-001 — Ten annual holidays
An active configured year contains exactly ten global holidays.

### HOL-002 — Eight-hour entitlement
Each configured holiday represents 8 hours; an unused year represents 80 Holiday hours.

### HOL-003 — Calendar applies globally
All ordinary users see the same configured holiday date/name for a year.

### HOL-004 — Holiday use
Taking a configured holiday off consumes exactly 8 Holiday hours.

### HOL-005 — Date restriction
Normal Holiday Use cannot be recorded against a non-holiday date.

### HOL-006 — Cannot consume twice
A configured holiday already used or converted by a user cannot be used/converted again.

### HOL-007 — Import failure is non-destructive
An invalid holiday import leaves the existing valid calendar intact.

## D. Floater conversion

### FLT-001 — Worked holiday creates one Floater
Working an unused configured holiday consumes/converts it and creates exactly 8 Floater hours.

### FLT-002 — Partial holiday work
Even partial work on the holiday creates exactly one 8-hour Floater under this application's simplified rule.

### FLT-003 — Multiple work events do not duplicate Floater
Multiple work records for the same holiday/user cannot create more than 8 Floater hours from that holiday.

### FLT-004 — Floater use
16 Floater minus 8 hours Use equals 8 Floater.

## E. Comp

### COMP-001 — User-confirmed credit
If the user confirms 6 credited Comp hours, Comp increases by exactly 6.

### COMP-002 — 1.5:1 example
Six hours worked with user-confirmed 9 credited Comp hours results in +9 Comp.

### COMP-003 — No inference
The system does not independently infer Comp credit/multiplier from time, weekday, weekend, or holiday.

### COMP-004 — Whole-hour Comp
The system does not automatically create fractional-hour Comp credits.

### COMP-005 — Notes retained
Comp work-event notes support and later display several sentences of context.

## F. Combined holiday work

### COMBO-001 — Holiday work with Comp and Floater
Given an unused configured holiday, a source work event with 6 hours worked, and user-confirmed +9 Comp:
- the holiday is consumed/converted;
- Floater increases by 8;
- Comp increases by 9;
- source event and effects are visibly related.

### COMBO-002 — Editing source event
Editing a source event keeps active generated effects consistent and preserves prior audit history.

### COMBO-003 — Deleting source event
Deleting a source event does not leave unexplained generated active effects; deletion remains auditable.

## G. Multi-day time off

### MULTI-001 — Deduct once
A 40-hour multi-day PTO entry deducts exactly 40 hours total.

### MULTI-002 — Calendar visualization
A multi-day entry is coherently represented over its applicable dates.

### MULTI-003 — Ledger coherence
A multi-day entry can be edited/deleted without orphaning or double-counting effects.

## H. Ledger and filtering

### LEDGER-001 — Newest first
Ledger defaults to descending chronological order for a selected year.

### LEDGER-002 — Year filter
Selected year displays only that year's effective records.

### LEDGER-003 — Bucket filter
Bucket filtering works and does not leak unrelated active records.

### LEDGER-004 — Classification filter
Filtering supports Grant, Use, Accrual, Conversion, Adjustment (or clearly equivalent labels).

### LEDGER-005 — Date range
Ledger can be filtered by date range.

### LEDGER-006 — Notes search
Free-text notes search finds only matching records owned by the authenticated user.

### LEDGER-007 — Dashboard drilldown
Clicking a Dashboard bucket opens Ledger filtered to that bucket.

## I. Dashboard and Calendar

### DASH-001 — Four cards
Dashboard shows PTO, Comp, Holiday, Floater.

### DASH-002 — Hours and days
Each card shows hours prominently and day equivalent secondarily.

### DASH-003 — Upcoming holidays
Dashboard shows useful upcoming holiday information.

### CAL-001 — Holidays visible
Configured holidays appear on Calendar.

### CAL-002 — Bucket visible
Calendar time-off records communicate which bucket was used.

### UI-001 — Mobile-first
Primary pages work without horizontal page scrolling at a common modern phone viewport.

### UI-002 — Desktop quality
Desktop layout uses space intentionally and remains polished.

### UI-003 — Dark mode
Dark mode is available and usable throughout primary flows.

## J. Authentication and isolation

### AUTH-001 — Self-registration
A user can register using unique username + password without email.

### AUTH-002 — Login
A registered user can authenticate using username + password.

### AUTH-003 — No cross-user read
Alice cannot retrieve Bob's records by UI manipulation, guessed IDs, direct requests, or altered parameters.

### AUTH-004 — No cross-user mutation
Alice cannot create/edit/delete Bob's records.

### AUTH-005 — No cross-user search
Alice's notes search cannot expose Bob's notes.

### AUTH-006 — Unauthenticated mutation denied
Unauthenticated clients cannot mutate protected data.

### AUTH-007 — Admin password reset
Admin can manually reset an ordinary user's password; the old password stops authenticating.

### AUTH-008 — Ordinary user not admin
Ordinary users cannot manage global holidays or reset another user's password.

### AUTH-009 — Password secrecy
Plaintext passwords never appear in DB, logs, or committed configuration.

## K. Edit/delete auditability

### AUDIT-001 — Edit preserves prior state
Current Ledger reflects edits while audit history can reconstruct the previous state.

### AUDIT-002 — Delete preserves evidence
Deleted record disappears from effective Ledger but remains auditable.

### AUDIT-003 — Audit isolation
Users cannot inspect another user's audit history.

### AUDIT-004 — Effective balance only
Historical revisions are not double-counted as active balance entries.

## L. Years

### YEAR-001 — Calendar year
Records are organized January 1 through December 31.

### YEAR-002 — Browse historical years
Prior years remain browseable while data exists.

### YEAR-003 — No automatic rollover
January 1 does not silently create PTO/Comp/Floater carryover.

### YEAR-004 — Explicit opening entry
Users can represent legitimate opening/carryover amounts via explicit Grant/Adjustment.

## M. Administration

### ADMIN-001 — Simple administrator role
One simple administrator role exists; generalized RBAC is unnecessary.

### ADMIN-002 — Holiday import
Admin can import/upload ten holidays for a selected year.

### ADMIN-003 — Holiday correction
Admin can inspect and correct imported holidays.

### ADMIN-004 — Account support
Admin can perform minimal account administration needed for manual password reset.

### ADMIN-005 — Local emergency administrator recovery
A local filesystem-authorized operator can recover the existing sole admin
password through an interactive, echo-disabled, double-confirmation command
holding exclusive database access. It selects the admin itself and rejects zero
or multiple admins, account selectors, noninteractive input, mismatch and invalid
passwords. It cannot promote or create an account and has no HTTP recovery path.

### AUTH-010 — Administrator recovery atomicity and isolation
Successful local recovery atomically replaces the admin password, revokes all
admin sessions and appends a secret-free operator audit event. The old password
and old sessions lose authority across restart; the new password authenticates.
Any recovery transaction failure preserves credentials, sessions and audit.
Ordinary-user credentials, sessions, settings, records and history remain intact.

## N. SQLite, backup, container

### DB-001 — Persistent SQLite
Replacing the app container while preserving mounted storage preserves user data.

### DB-002 — Migrations
A clean DB can be initialized through documented migrations/schema setup.

### DB-003 — Consistent backup
Backup tooling creates a recoverable SQLite backup without unsafe raw-copy assumptions.

### DB-004 — Restore
A documented restore into a test deployment recovers expected ledger/user data.

### DB-005 — Backup storage
Backups are not stored solely in disposable container layers.

## O. Deployment boundary

### DEPLOY-001 — Container smoke test
Documented local/test container workflow successfully starts the app.

### DEPLOY-002 — No real deployment
Development/review tasks never deploy to the real VPS or alter real DNS.

### DEPLOY-003 — Human-usable deployment
Operator docs/scripts allow a human to configure, start, update, back up, and restore on Linux VPS.

## P. Forbidden identity leakage

### BRAND-001 — Forbidden employer identity absent
The employer/company identity that inspired the rules does not appear in source, comments, docs, tests, fixtures, UI, naming, or deployment examples.

If an automated repository check enforces this invariant, it must do so without committing the forbidden identity itself.
