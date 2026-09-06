# DOMAIN.md

## Scope and terminology

This document defines authoritative business behavior.

The product tracks four fixed time-off buckets:
- PTO
- Comp
- Holiday
- Floater

All ledger amounts are whole hours.

The display conversion is:

`1 day = 8 hours`

Balances must never be negative.

The application is organized by calendar year.

## Ledger principle

Balances must be explainable from recorded activity.

A balance should not exist as an unexplained manually overwritten number.

The implementation may cache or materialize balances for performance, but the authoritative business history must be reconstructable from source records and ledger/audit information.

Supported transaction concepts include:
- Grant
- Use
- Accrual
- Conversion
- Adjustment

Agents may implement more precise internal types when useful.

## User ownership

Every personal record belongs to exactly one user unless it is explicitly global configuration such as the annual holiday calendar.

Ordinary users may only view or mutate their own personal data.

The administrator may manage global holiday configuration and perform limited account-administration functions such as password reset.

## PTO

PTO entitlement rules based on years of service are intentionally not encoded in v1.

Each user configures/records their own applicable PTO amount.

General policy:
- annual PTO is treated as available for the year rather than gradually accrued;
- up to two work weeks may carry into the following year;
- two work weeks equals 80 hours.

v1 does not automatically:
- calculate entitlement from hire date;
- create the January 1 PTO grant;
- calculate carryover;
- enforce the user's correct carryover amount;
- transfer balances between years.

Users represent the correct new-year opening amount through explicit ledger entries such as grants/adjustments.

PTO usage reduces PTO by the recorded number of whole hours.

PTO may not fall below zero.

## Global annual holiday calendar

Each calendar year has exactly ten configured holidays.

Each holiday represents 8 hours.

Actual holiday names/dates may change each year.

The administrator must be able to import/upload the holiday list for a year and correct it when necessary.

The holiday calendar applies globally to all users.

The import file format and exact admin UX are implementation decisions, but the resulting configuration must be easy to inspect and correct.

## Holiday

A user's annual Holiday availability is defined by the ten globally configured holidays for that year:

`10 holidays × 8 hours = 80 hours`

Holiday time is tied to its configured holiday date.

Normal holiday use:
- taking the configured holiday off consumes 8 hours of Holiday;
- Holiday may not be used on an arbitrary non-holiday date.

Working a configured holiday:
- consumes/converts that holiday entitlement;
- creates one Floater worth 8 hours;
- this Floater rule applies to every configured holiday in this application.

A configured holiday should not be consumed twice for the same user.

The implementation must prevent one holiday from generating multiple Floaters for the same user merely because multiple work records exist on that holiday.

If a user works only part of the holiday, the application's simplified policy still treats the holiday as worked for Floater conversion:
- the holiday entitlement is converted;
- exactly one 8-hour Floater is created.

Any Comp credit related to that work is recorded separately.

Holiday balance may not fall below zero.

## Floater

A Floater behaves like flexible paid time off once created.

One worked configured holiday creates:

`+8 hours Floater`

Exactly one Floater may be created per configured holiday per user.

The amount does not scale with the number of hours worked that day.

Floater use reduces the Floater balance by the recorded whole hours.

Floater may not fall below zero.

v1 performs no automatic year-to-year Floater carryover. If a balance is legitimately carried into another year, the user records the appropriate opening grant/adjustment.

## Comp

Comp represents time earned for qualifying work outside normal working expectations.

The application does not attempt to determine whether work qualifies.

The user is responsible for determining the appropriate credit.

Common real-world cases include:
- normal qualifying after-hours/weekend work: typically 1:1;
- qualifying holiday work: typically 1.5:1.

The application must not infer the multiplier from clock time, day of week, or holiday status.

The user may record whether an event was treated as 1:1 or 1.5:1 for context/auditability, but the credited Comp amount is user-confirmed.

Because v1 ledger precision is whole hours, the application must not create fractional-hour Comp automatically. If a mathematical multiplier would result in a fractional hour, the user remains responsible for entering the correct whole-hour credited amount according to their real-world practice.

A Comp-generating work event should be able to retain:
- event date;
- hours worked, when entered;
- applicable classification/multiplier context, when entered;
- final Comp hours credited;
- notes.

Example:

A user records six hours of qualifying holiday work and confirms a 1.5:1 treatment.

The associated Comp credit may be:

`+9 hours Comp`

The holiday-work event may separately trigger the one-time 8-hour Floater conversion defined above.

Comp use reduces Comp by the recorded whole hours.

Comp may not fall below zero.

v1 defines no automatic Comp expiration, maximum balance, or year-to-year carryover processing. Any real-world adjustment is represented explicitly by the user.

## Taking time off

At minimum, a use entry records:
- date or date range;
- bucket;
- whole hours used;
- notes;
- classification as a use transaction.

Users may use multiple buckets across the same day when desired, provided each ledger effect is explicit and no bucket becomes negative.

Multi-day usage is allowed.

The representation must prevent duplicate deduction. A 40-hour multi-day vacation deducts exactly 40 hours total, regardless of how many calendar cells/visual elements represent it.

## Source events and generated effects

The application should preserve a comprehensible relationship between a source event and resulting ledger effects.

Example:

Source event:
- worked configured holiday;
- 6 hours worked;
- user-confirmed 1.5:1 Comp treatment;
- notes explaining the incident.

Resulting effects:
- Holiday entitlement for that date converted/consumed;
- `+8 hours Floater`;
- `+9 hours Comp`.

The user should be able to understand that these entries belong to one real-world event.

Implementation details are left to the agents.

## Editing, deletion, and audit history

Users must be able to edit and delete their own records directly through the product.

However, edits/deletions must not erase historical accountability.

The system must preserve enough audit information to determine:
- what record existed before an edit;
- what values changed;
- when a record was deleted;
- the relationship between a changed source event and any generated ledger effects.

The primary Ledger should show current effective records by default; historical revisions/deletions may be surfaced through transaction details or another unobtrusive audit-history view.

Modification timestamps are allowed and expected if useful.

An edit that changes a source event must keep generated effects consistent.

A deletion that removes a source event must not leave unexplained generated credits/debits active.

## Year boundaries

Each year is logically independent for reporting/browsing.

Historical years must remain unchanged unless the user deliberately edits historical records.

No scheduled January 1 job is required to:
- grant PTO;
- roll PTO;
- roll Comp;
- roll Floater.

Users enter new-year opening grants/adjustments manually.

The annual global holiday calendar is the exception: configuring the ten holidays for a year establishes that year's Holiday entitlement/calendar structure for all users.

## No negative balances

This invariant applies to:
- PTO
- Comp
- Holiday
- Floater

Any create/edit/delete/conversion operation that would make an effective balance negative must be rejected atomically with a useful user-facing error.

Partial application is not allowed.
