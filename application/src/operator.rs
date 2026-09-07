//! Emergency local recovery. No HTTP handler or App method exposes this path.
use crate::{
    Clock, SystemClock,
    db::Db,
    error::{ApiError, Result},
    security,
};
use rusqlite::{TransactionBehavior, params};
use std::{io::IsTerminal, path::Path};
use zeroize::Zeroizing;

/// Requires a real interactive terminal and exclusive local database access.
/// There is deliberately no account selector or supplied-password parameter.
pub fn reset_admin_password(path: impl AsRef<Path>) -> Result<()> {
    recover_with_prompt(path.as_ref(), &mut Terminal)
}

// Private test seam; production callers cannot replace the terminal gate or
// supply secrets through arguments, environment, stdin pipes or HTTP.
trait Prompt {
    fn interactive(&self) -> bool;
    fn password(&mut self, message: &str) -> Result<Zeroizing<String>>;
}
struct Terminal;
impl Prompt for Terminal {
    fn interactive(&self) -> bool {
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
    }
    fn password(&mut self, message: &str) -> Result<Zeroizing<String>> {
        rpassword::prompt_password(message)
            .map(Zeroizing::new)
            .map_err(|_| ApiError::bad())
    }
}
fn recover_with_prompt(path: &Path, prompt: &mut impl Prompt) -> Result<()> {
    if !prompt.interactive() {
        return Err(ApiError::bad());
    }
    // Like bootstrap, validate terminal input before opening or upgrading a DB.
    let password = prompt.password("New administrator password (12–128 bytes): ")?;
    let repeated = prompt.password("Repeat password: ")?;
    if !security::equal_secret(&password, &repeated) {
        return Err(ApiError::bad());
    }
    security::password(&password)?;
    // Recovery must not initialize a mistyped/missing database path.
    if !path.metadata().is_ok_and(|m| m.is_file()) {
        return Err(ApiError::bad());
    }
    let mut db = Db::open(path)?;
    let hash = Zeroizing::new(security::hash(&password)?);
    let tx = db
        .identity
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let admins: Vec<Vec<u8>> = tx
        .prepare("SELECT id FROM accounts WHERE admin=1 LIMIT 2")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let [id] = admins.as_slice() else {
        return Err(ApiError::bad());
    };
    if id.len() != 8 {
        return Err(ApiError::bad());
    }
    let changed = tx.execute(
        "UPDATE accounts SET password_hash=?1 WHERE id=?2 AND admin=1",
        params![hash.as_str(), id],
    )?;
    if changed != 1 {
        return Err(ApiError::bad());
    }
    tx.execute("DELETE FROM sessions WHERE owner=?1", [id])?;
    tx.execute(
        "INSERT INTO admin_recovery_history(target,actor,at) VALUES (?1,'local_operator',?2)",
        params![id, SystemClock.now()],
    )?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/admin_recovery/mod.rs"]
mod admin_recovery;
