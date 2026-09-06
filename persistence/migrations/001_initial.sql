CREATE TABLE calendars (
    year INTEGER PRIMARY KEY CHECK(year BETWEEN 1 AND 9999),
    revision INTEGER NOT NULL CHECK(revision > 0),
    payload TEXT NOT NULL CHECK(json_valid(payload))
) STRICT;
CREATE TABLE holidays (
    year INTEGER NOT NULL REFERENCES calendars(year),
    id BLOB NOT NULL CHECK(length(id) = 8),
    date TEXT NOT NULL,
    name TEXT NOT NULL,
    PRIMARY KEY(year, id),
    UNIQUE(year, date)
) STRICT;
CREATE TABLE sources (
    owner BLOB NOT NULL CHECK(length(owner) = 8),
    id BLOB NOT NULL CHECK(length(id) = 8),
    revision BLOB NOT NULL CHECK(length(revision) = 8 AND revision != zeroblob(8)),
    year INTEGER NOT NULL CHECK(year BETWEEN 1 AND 9999),
    holiday_id BLOB,
    payload TEXT CHECK(payload IS NULL OR json_valid(payload)),
    PRIMARY KEY(owner, id),
    UNIQUE(owner, id, revision, year),
    FOREIGN KEY(year, holiday_id) REFERENCES holidays(year, id) DEFERRABLE INITIALLY DEFERRED,
    CHECK(payload IS NOT NULL OR holiday_id IS NULL)
) STRICT;
CREATE INDEX sources_owner_year ON sources(owner, year);
CREATE INDEX sources_holiday ON sources(year, holiday_id) WHERE payload IS NOT NULL;
CREATE TABLE effects (
    owner BLOB NOT NULL CHECK(length(owner) = 8),
    year INTEGER NOT NULL CHECK(year BETWEEN 1 AND 9999),
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
    payload TEXT NOT NULL CHECK(json_valid(payload)),
    PRIMARY KEY(owner, year, ordinal)
) STRICT;
CREATE TABLE effect_supports (
    owner BLOB NOT NULL,
    year INTEGER NOT NULL,
    ordinal INTEGER NOT NULL,
    source_id BLOB NOT NULL,
    revision BLOB NOT NULL,
    PRIMARY KEY(owner, year, ordinal, source_id),
    FOREIGN KEY(owner, year, ordinal) REFERENCES effects(owner, year, ordinal) ON DELETE CASCADE,
    FOREIGN KEY(owner, source_id, revision, year) REFERENCES sources(owner, id, revision, year)
        DEFERRABLE INITIALLY DEFERRED
) STRICT;
CREATE TABLE source_history (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    owner BLOB NOT NULL,
    source_id BLOB NOT NULL,
    at INTEGER NOT NULL,
    payload TEXT NOT NULL CHECK(json_valid(payload)),
    FOREIGN KEY(owner, source_id) REFERENCES sources(owner, id)
) STRICT;
CREATE INDEX history_owner_source ON source_history(owner, source_id, sequence);
CREATE TABLE calendar_history (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    year INTEGER NOT NULL REFERENCES calendars(year),
    revision INTEGER NOT NULL CHECK(revision > 0),
    at INTEGER NOT NULL,
    payload TEXT NOT NULL CHECK(json_valid(payload)),
    UNIQUE(year, revision)
) STRICT;
CREATE TRIGGER source_history_no_update BEFORE UPDATE ON source_history
BEGIN SELECT RAISE(ABORT, 'immutable source history'); END;
CREATE TRIGGER source_history_no_delete BEFORE DELETE ON source_history
BEGIN SELECT RAISE(ABORT, 'immutable source history'); END;
CREATE TRIGGER calendar_history_no_update BEFORE UPDATE ON calendar_history
BEGIN SELECT RAISE(ABORT, 'immutable calendar history'); END;
CREATE TRIGGER calendar_history_no_delete BEFORE DELETE ON calendar_history
BEGIN SELECT RAISE(ABORT, 'immutable calendar history'); END;
