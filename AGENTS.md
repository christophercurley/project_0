# AGENTS.md

## Purpose

This repository is an agentic-development experiment for a small, production-quality time-off tracking web application.

Before making architectural or behavioral decisions, read the authoritative documents under `docs/`.

## Authority order

When documents appear to conflict, use this precedence:

1. `docs/DOMAIN.md`
2. `docs/ACCEPTANCE.md`
3. `docs/ARCHITECTURE.md`
4. `docs/PRODUCT.md`

Do not silently reinterpret product or domain rules to simplify implementation.

If a material requirement is genuinely ambiguous or contradictory, document the issue and stop at the nearest safe boundary rather than inventing business behavior.

## Hard constraints

- Backend: Rust.
- Async/runtime/server: Tokio + Axum.
- Frontend: static HTML, CSS, and vanilla JavaScript.
- Do not introduce React, Vue, Svelte, Angular, Tailwind, Bootstrap, or another frontend framework/CSS framework.
- Persistence: SQLite.
- The application must be containerized with Docker.
- The application must support multiple independent users from day one.
- User data must be strictly isolated by authenticated user.
- Open self-registration is required.
- Authentication is username + password only.
- Do not add email, email verification, email password reset, OAuth, SSO, or 2FA.
- One administrator role exists for account administration and manual password resets.
- The employer/company name that motivated this application must never appear anywhere in the repository, UI, documentation, source comments, test fixtures, sample data, deployment files, generated application name, or hostname examples.
- Agents must never deploy to a real server. Produce deployment artifacts/scripts only.
- Production credentials, real secrets, and real user data must never be committed.
- Source time-off data is user-owned and must not be exposed across accounts.
- Ledger balances may never become negative.
- Source transactions must be editable/deletable through the product, but historical change information must remain auditable.

## Engineering expectations

All completed work must:

- build successfully;
- pass the complete automated test suite;
- pass `cargo fmt --check`;
- pass Clippy with warnings denied for project code;
- include tests for new domain behavior;
- preserve user isolation;
- preserve documented audit behavior;
- avoid unrelated refactors unless they are necessary and justified;
- update relevant documentation when a stable implementation or operational decision is introduced.

Prefer straightforward, maintainable designs over speculative abstractions.

Do not weaken, delete, rewrite, or bypass a valid test merely to make a change pass. A test may be modified when:
- the requirement changed in authoritative documentation;
- the test is demonstrably incorrect;
- the test is flaky or invalid for a documented technical reason.

Any such modification must be justified in the task summary or review.

## Security baseline

This is a small internet-facing application. Do not over-engineer identity, but do implement ordinary modern web-security hygiene appropriate to password/session authentication, including secure password hashing, secure session handling, input validation, authorization checks, and reasonable login/registration abuse resistance.

## Database and storage

SQLite data must live on persistent storage mounted outside disposable container layers.

Backup tooling must create recoverable SQLite backups and provide a simple way for the operator to copy/retain backups elsewhere on the same VPS. Container rebuild/replacement must not destroy application data.

## Naming

The project does not yet have a final product name.

During the planning gate, propose a concise modern product name and an optional short trigram suitable for a hostname such as `<name-or-trigram>.madscience.lol`.

The name must not reference or imply the employer/company that inspired the domain rules.

Do not rename the repository or assume DNS/deployment changes have been approved.

## Agentic workflow

### Planning gate

Before implementation, inspect all authoritative documents and produce a written implementation plan covering:

- proposed product name;
- system architecture;
- domain-model approach;
- SQLite persistence approach;
- authentication/session approach;
- audit-history approach;
- frontend approach;
- test strategy;
- deployment/backup strategy;
- milestone sequence;
- major technical risks;
- any genuine requirement ambiguity.

Do not modify production code during this planning task.

### Milestone 1

Milestone 1 is intentionally constrained to the pure domain model and its automated test harness.

Do not build the web UI, Axum routes, SQLite persistence, authentication, Docker image, or deployment scripts in Milestone 1 unless a tiny amount of scaffolding is strictly required to test the domain crate/module.

The goal is to prove the business rules independently of infrastructure.

### Reviews

After an implementation milestone completes, a reviewer agent may:
- inspect all code and tests;
- add adversarial tests;
- add missing tests;
- modify an incorrect test with written justification;
- report defects and architectural concerns.

Reviewer agents should assume the implementation may be subtly wrong. Existing passing tests are evidence, not proof.

Implementation agents should address justified findings and rerun the complete validation suite.

Human review happens after automated validation and reviewer-agent findings have been resolved.
