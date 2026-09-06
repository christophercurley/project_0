# CODEX_RUNBOOK.md

## Recommended working model

Use Codex against a local clone/worktree and keep GitHub as the durable remote/source of truth.

The local repository is where Codex can inspect/edit code and run Cargo, tests, Docker, branches, and worktrees.

GitHub provides durable history, branches, pull requests, review history, and later CI.

The repository can begin under a temporary neutral name such as `project-zero`. The final name can be chosen after the planning gate.

## Bootstrap

Create an empty local repository, copy this handoff package into it, and commit the documentation before asking Codex to implement.

Example:

```bash
mkdir project-zero
cd project-zero
git init

# copy AGENTS.md, CODEX_RUNBOOK.md and docs/ here

git add .
git commit -m "Add project handoff specification"
```

Then create a private GitHub repository/remote using your normal GitHub workflow and push the initial commit.

## Gate 1 — Planning only

Run Codex from the repository root.

Suggested prompt:

> Read AGENTS.md and every authoritative document under docs/. Do not implement application code yet. Produce a concrete implementation plan for the complete product. Include a proposed product name and optional trigram, architecture, domain-model approach, SQLite persistence design, authentication/session design, audit-history design, frontend approach, testing strategy, backup/deployment strategy, milestone sequence, major risks, and every genuinely ambiguous requirement you believe blocks correct implementation. Preserve all hard constraints. Do not silently invent business rules.

Human action:
- inspect the plan;
- reject needless complexity;
- answer only genuine product/domain ambiguities;
- approve or amend the milestone plan.

Do not prescribe low-level code organization unless necessary.

## Gate 2 — Milestone 1 implementation

Milestone 1 is the pure domain model and test harness.

Suggested prompt:

> Implement Milestone 1 according to AGENTS.md and the authoritative docs. Build the pure Rust domain model and a comprehensive automated test harness for the documented business rules and invariants. Do not build the frontend, Axum API, SQLite persistence, authentication, Docker deployment, or production tooling yet except for minimal scaffolding strictly necessary to test the domain code. You own implementation decisions. Run all applicable formatting, linting, and tests before declaring the milestone complete. Summarize what you implemented, tests executed, remaining risks, and any requirement ambiguity discovered.

Do not intervene merely because Codex encounters compiler/test failures. Let the implementation agent iterate.

## Gate 3 — Independent review

Start a separate Codex review context/task against the completed Milestone 1 branch/worktree.

Suggested prompt:

> You are the independent reviewer for Milestone 1. You did not implement this code. Read AGENTS.md and all authoritative docs, then inspect the implementation and tests. Assume the implementation may be subtly wrong. Attempt to falsify correctness. Add adversarial or missing tests where useful. You may modify an existing test only when it is demonstrably incorrect, invalid, or inconsistent with authoritative requirements, and you must justify that modification. Do not weaken tests merely to obtain a pass. Report domain-rule violations, design risks, missing edge cases, and test gaps. Run the relevant validation suite.

## Gate 4 — Review remediation

Suggested prompt:

> Review the independent Milestone 1 findings and code/test changes. Resolve every justified issue without weakening documented requirements. If you reject a finding, explain specifically why it conflicts with the authoritative docs or is technically invalid. Run the complete Milestone 1 validation suite afterward and provide a final milestone summary.

## Human milestone review

Only after:
- implementation tests pass;
- formatter/linter checks pass;
- reviewer work is complete;
- justified review findings are resolved;

should the human inspect the milestone.

Review requirements, architecture, behavior, and significant design choices rather than personally debugging each compiler error or function.

## Later milestones

The planning agent may refine the sequence, but a sensible default is:

1. Pure domain model + exhaustive tests.
2. SQLite schema/persistence + audit model + integration tests.
3. Authentication, authorization, admin functions, Axum API + security/isolation tests.
4. Mobile-first static frontend: Dashboard, Ledger, Calendar.
5. Full-stack/browser acceptance testing and visual/UX refinement.
6. Docker packaging, backup/restore validation, operator deployment scripts/docs.
7. Release-candidate hostile review/security/performance pass.

Parallel agents/worktrees become more useful after domain and persistence contracts stabilize.

Potential later split:
- backend/persistence agent;
- frontend agent;
- test/adversarial agent;
- security reviewer;
- release/integration agent.

## Production rule

No Codex task should receive real VPS credentials or authority to deploy.

The final deployment milestone ends with:
- locally tested artifacts;
- operator scripts;
- backup/restore proof;
- deployment documentation.

A human performs the real deployment.
