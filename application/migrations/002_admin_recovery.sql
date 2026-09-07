CREATE TABLE admin_recovery_history (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    target BLOB NOT NULL REFERENCES accounts(id),
    actor TEXT NOT NULL CHECK(actor = 'local_operator'),
    at INTEGER NOT NULL
) STRICT;
CREATE TRIGGER admin_recovery_history_no_update BEFORE UPDATE ON admin_recovery_history
BEGIN SELECT RAISE(ABORT, 'immutable administrator recovery history'); END;
CREATE TRIGGER admin_recovery_history_no_delete BEFORE DELETE ON admin_recovery_history
BEGIN SELECT RAISE(ABORT, 'immutable administrator recovery history'); END;
