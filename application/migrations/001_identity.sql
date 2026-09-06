CREATE TABLE accounts (
    id BLOB PRIMARY KEY CHECK(length(id)=8),
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    admin INTEGER NOT NULL CHECK(admin IN (0,1)),
    created_at INTEGER NOT NULL,
    settings TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(settings))
) STRICT;
CREATE UNIQUE INDEX one_initial_admin ON accounts(admin) WHERE admin=1;
CREATE TABLE sessions (
    token_hash BLOB PRIMARY KEY CHECK(length(token_hash)=32),
    owner BLOB NOT NULL REFERENCES accounts(id),
    csrf TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    last_seen INTEGER NOT NULL
) STRICT;
CREATE INDEX sessions_owner ON sessions(owner, last_seen);
CREATE INDEX sessions_expiry ON sessions(expires_at);
CREATE TABLE account_history (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    actor BLOB NOT NULL REFERENCES accounts(id),
    target BLOB NOT NULL REFERENCES accounts(id),
    action TEXT NOT NULL CHECK(action IN ('registered','bootstrapped','password_reset','settings_changed')),
    at INTEGER NOT NULL
) STRICT;
CREATE TRIGGER account_history_no_update BEFORE UPDATE ON account_history
BEGIN SELECT RAISE(ABORT, 'immutable account history'); END;
CREATE TRIGGER account_history_no_delete BEFORE DELETE ON account_history
BEGIN SELECT RAISE(ABORT, 'immutable account history'); END;
CREATE TRIGGER accounts_no_delete BEFORE DELETE ON accounts
BEGIN SELECT RAISE(ABORT, 'accounts cannot be deleted'); END;
CREATE TRIGGER accounts_identity_immutable BEFORE UPDATE OF id,username,admin ON accounts
BEGIN SELECT RAISE(ABORT, 'account identity cannot be changed'); END;
CREATE TRIGGER sources_account_insert BEFORE INSERT ON sources
WHEN NOT EXISTS(SELECT 1 FROM accounts WHERE id=NEW.owner)
BEGIN SELECT RAISE(ABORT, 'unknown account'); END;
CREATE TRIGGER sources_account_update BEFORE UPDATE OF owner ON sources
WHEN NEW.owner != OLD.owner
BEGIN SELECT RAISE(ABORT, 'source owner cannot be changed'); END;
CREATE TABLE auth_limits (
    key TEXT PRIMARY KEY,
    started INTEGER NOT NULL,
    attempts INTEGER NOT NULL
) STRICT;
