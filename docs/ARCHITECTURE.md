# ARCHITECTURE.md

## Goal

Define hard technical boundaries while leaving meaningful implementation freedom to the development agents.

This document intentionally does not prescribe file names, Rust structs, endpoint paths, JavaScript module layout, or database table names.

## Required technology

### Backend

- Rust
- Tokio
- Axum

Agents may select supporting crates as needed.

Prefer a modest dependency graph and well-maintained libraries.

### Frontend

- static HTML
- CSS
- vanilla JavaScript

Do not use:
- React
- Vue
- Svelte
- Angular
- Tailwind
- Bootstrap
- another frontend/UI framework

A small first-party CSS/JS component/utilities layer is encouraged.

### Persistence

- SQLite

The database must be stored on persistent mounted storage outside the disposable Docker image/container layer.

Agents may choose the Rust SQLite library/ORM/query layer.

Database migrations/schema versioning are required.

SQLite should be configured appropriately for a small multi-user web application with safe concurrent access.

### Deployment

- Dockerized application
- Linux VPS target
- eventual hostname: `<approved-name-or-trigram>.madscience.lol`

Agents prepare:
- Dockerfile(s);
- compose/run configuration if useful;
- environment/configuration examples;
- deployment/update scripts;
- backup scripts;
- restore instructions;
- health-check guidance;
- operator documentation.

Agents never deploy to the real VPS.

## Application shape

A single deployable application is preferred.

Do not introduce microservices, distributed queues, Kubernetes, or external infrastructure without a compelling documented reason.

The server may serve the static frontend itself.

## Domain isolation

Business rules should be testable without requiring:
- a browser;
- a running Axum server;
- SQLite;
- Docker.

The pure domain layer should not be tightly coupled to HTTP or persistence concerns.

Milestone 1 must demonstrate this property.

## Authentication and sessions

Required user experience:
- open self-registration;
- username + password login;
- no email;
- no user-facing password reset;
- no OAuth/SSO;
- no 2FA.

Passwords must use a modern password-hashing algorithm appropriate for interactive logins.

Authentication should use secure server-managed sessions or an equivalently appropriate same-origin web approach.

Session cookies/settings must be appropriate for an HTTPS internet-facing site.

The design must defend against:
- user A accessing user B's records;
- IDOR-style object access;
- unauthenticated mutation;
- common session/authentication mistakes;
- obvious automated registration/login abuse.

Do not add CAPTCHA unless justified by actual need; reasonable rate limiting/throttling is preferable for v1.

## Administrator

The system has exactly one administrator account and one administrator role.

Admin capabilities in v1 are intentionally narrow:
- annual global holiday-calendar management;
- inspect basic user account list/status as needed for administration;
- manually set/reset another user's password.

Admin does not need a generalized RBAC framework.

The initial admin bootstrap method is an implementation decision and must be documented securely.

Loss of the sole administrator password is recoverable only through a local
operator command with filesystem authority and the same exclusive canonical
database lock used by bootstrap. It requires interactive terminal input with
echo disabled and matching confirmation, using the existing password policy
and Argon2id implementation. The command selects exactly one existing admin
itself; it accepts no account selector and cannot create or promote accounts.
Password replacement, revocation of every admin session, and a secret-free
operator recovery audit append must commit atomically or roll back together.
Ordinary-user credentials, sessions and data remain unchanged. This emergency
operator mechanism has no HTTP, browser, email, token or remote recovery path.

## Audit history

User-facing edit/delete operations are required.

Persistence must still retain historical revisions/deletions sufficiently to satisfy `DOMAIN.md`.

Agents may choose:
- append-only audit events;
- version tables;
- soft deletion plus revision history;
- another simple reliable design.

Avoid a complex event-sourcing architecture unless clearly justified.

## Global holiday configuration

The annual holiday calendar is global site configuration.

It contains exactly ten holidays for a configured year.

The administrator needs an upload/import workflow and the ability to inspect/correct the imported result.

The import representation/file format is an implementation decision and must be documented.

Invalid imports must fail safely without partially corrupting the active calendar.

## API

The HTTP/API shape is an implementation decision.

Requirements:
- coherent same-origin interface for the static frontend;
- clear input validation;
- correct status/error behavior;
- strict authorization;
- no cross-user leakage;
- testability.

Avoid designing a public third-party API unless needed internally.

## UI responsiveness

The application is mobile-first.

Desktop layouts should make good use of additional space without becoming a separate design.

Dark mode is required.

The application should feel contemporary and intentionally designed.

Accessibility basics such as labels, keyboard usability, focus states, contrast, semantic controls, and reduced-motion respect should be included.

## Time representation

Business quantities are whole hours.

Do not use floating-point numbers for authoritative ledger quantities.

Day equivalents are presentation values based on 8 hours/day.

Dates are calendar dates within the user's selected year.

The application is not required to model exact work-event start/end timestamps unless the chosen UX finds them useful. Do not infer Comp qualification from timestamps.

## Consistency and atomicity

Multi-effect operations must be atomic.

For example, recording a holiday-work conversion that creates:
- holiday consumption/conversion;
- a Floater;
- a Comp credit

must not leave a partially applied state if validation or persistence fails.

Edits/deletions that alter generated effects have the same requirement.

## Testing

Testing should include, at minimum:

- pure domain unit tests;
- property/invariant tests where valuable;
- SQLite integration tests using isolated temporary databases;
- authentication/authorization tests;
- cross-user isolation tests;
- HTTP/API integration tests;
- frontend/browser-level acceptance tests for critical user flows;
- migration tests;
- backup/restore validation;
- container smoke test.

The test suite should favor deterministic local execution.

Agents are encouraged to add adversarial tests.

## Observability

Use structured application logging suitable for operating the service on a VPS.

Do not log:
- plaintext passwords;
- password hashes;
- session secrets/tokens;
- unnecessary sensitive ledger-note contents.

Provide a simple health endpoint or equivalent health mechanism suitable for container monitoring.

## Backups

Automated SQLite backups are required.

Requirements:
- backup must be consistent/recoverable;
- backup output must live outside disposable container layers;
- retention behavior should be simple and documented;
- provide an operator script/workflow to copy/retain backups elsewhere on the VPS;
- include a documented restore procedure;
- test the backup/restore path before calling the deployment milestone complete.

## Secrets/configuration

Secrets belong in environment variables or mounted secret/config mechanisms, never source control.

Provide a safe example environment file with placeholder values only.

## Production boundary

No agent may:
- SSH to the real VPS;
- alter DNS;
- create production users;
- deploy containers;
- touch a production database;
- install cron/systemd jobs on the real host.

Agents create and test the artifacts locally. A human performs production deployment.
