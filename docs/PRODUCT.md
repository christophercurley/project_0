# PRODUCT.md

## Product summary

Build a small, polished, multi-user web application for tracking personal paid time-off balances and the history behind those balances.

The application exists because users are responsible for maintaining their own records across several distinct time-off buckets. The core value is not merely showing a balance; it is being able to answer:

- How much time remains?
- Where did the balance come from?
- When was time earned, granted, converted, or used?
- What happened on a particular date?
- What does the entire year look like at a glance?

The application is intentionally purpose-built for one employer's time-off practices. It is not intended to become a generic HR platform.

The employer/company name must never appear anywhere in the product or repository.

## Primary users

The application must support multiple independent users from day one.

Most users are ordinary users who:
- self-register with a username and password;
- configure their own personal settings such as hire date and annual PTO allowance;
- maintain their own ledger;
- view their own calendar and dashboard;
- never see another user's private time-off data.

One administrator role exists for:
- maintaining the global annual holiday calendar;
- basic account administration;
- manually resetting a user's password when necessary.

There is no email subsystem.

## Core time-off buckets

The product tracks exactly four first-class buckets:

1. PTO
2. Comp
3. Holiday
4. Floater

All balances are tracked in hours.

The UI must also show a friendly day equivalent using:

`8 hours = 1 day`

Ledger quantities are whole hours.

## Primary navigation

The product has three primary user areas.

### Dashboard

The dashboard is the quick status view.

It must prominently show one card for each bucket:
- PTO
- Comp
- Holiday
- Floater

Each card shows:
- hours remaining;
- a smaller but clearly readable equivalent in days.

Clicking a bucket card opens the Ledger already filtered to that bucket.

The dashboard may include a compact "last five transactions" section if it improves the experience.

The dashboard should show upcoming holidays from the site's configured holiday calendar.

Do not add charts in v1.

### Ledger

The Ledger is the authoritative human-readable history for a selected calendar year.

Default ordering:
- newest first.

Users can browse previous years indefinitely.

Filtering must support:
- year;
- bucket;
- transaction classification;
- date range;
- free-text search over notes.

Users must be able to add, edit, and delete their own entries through a clear UI.

Where one source event creates generated ledger effects, the relationship should be understandable. Example: a holiday work event may be associated with Comp credit and Floater creation.

No CSV/JSON export is required in v1.

### Calendar

The Calendar provides a visual yearly/monthly view of time-off activity.

It should make it easy to see:
- holidays;
- days on which time was used;
- which bucket funded time off;
- multi-day vacation/time-off periods;
- work events or conversions where useful without overwhelming the calendar.

Multi-day entries should look coherent and pleasant rather than appearing as unrelated records.

## Transaction entry

A time-off or ledger entry needs, at minimum:

- date or date range;
- bucket;
- hours;
- notes;
- transaction classification.

User-facing terminology may improve on "transaction classification", but the underlying concept must distinguish cases such as:

- Grant
- Use
- Accrual
- Conversion
- Adjustment

Agents may refine labels and UX so long as domain meaning remains clear.

Notes are especially valuable for Comp-generating events and should support a few sentences of context.

## Multi-day entries

Users should be able to record multi-day time off conveniently.

The implementation must avoid ambiguous accounting. The UI/data model may choose an appropriate representation, but:
- total deducted hours must be unambiguous;
- calendar visualization must represent the included dates correctly;
- ledger history must avoid accidental double-counting.

## Annual model

The application is organized by calendar year:

January 1 through December 31.

Prior years remain browseable indefinitely.

v1 does not automatically create annual PTO grants or automatically carry balances into the next year. Users explicitly record appropriate opening grants/adjustments for a new year.

The global holiday calendar is configured annually by the administrator and defines the site's ten holidays for that year.

## Visual direction

The interface must be:

- mobile-first;
- fully responsive on desktop;
- modern and contemporary;
- visually impressive without becoming flashy or cluttered;
- usable in dark mode.

The frontend must be written with static HTML, CSS, and vanilla JavaScript.

A small project-specific UI style/component layer is encouraged.

Do not use Bootstrap, Tailwind, or a JavaScript UI framework.

Navigation style (hamburger, responsive top navigation, etc.) is an implementation/design decision.

## Notifications

Do not build notifications in v1.

No:
- email notifications;
- push notifications;
- reminder engine;
- SMS;
- expiring-balance alerts.

Upcoming holidays may be shown passively on the Dashboard.

## Deployment expectation

The finished product will eventually run on a Linux VPS at a subdomain of:

`madscience.lol`

The exact subdomain depends on the approved product name/trigram.

Agents must not deploy to the real VPS.

The repository must instead provide clear, simple deployment artifacts and operator instructions.

The app should run as a Docker container and use SQLite persisted through mounted storage.

Automated backup tooling is required.

## Explicitly out of scope for v1

- employer integrations;
- HR system integrations;
- email;
- 2FA;
- OAuth/SSO;
- user-facing password recovery;
- automatic PTO entitlement calculation from years of service;
- automatic annual PTO grant creation;
- automatic carryover processing;
- charts;
- data migration from any previous application;
- exports;
- generalized multi-company policy engines;
- production deployment performed by an agent.
