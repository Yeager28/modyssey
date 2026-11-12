//! Logging in to Steam, and reconnecting from a saved login.

use super::store::{now_secs, LoginStore, SavedLogin};
use anyhow::{anyhow, bail, Context, Result};
use steam_vent::auth::{
    AuthConfirmationHandler, ConsoleAuthConfirmationHandler, DeviceConfirmationHandler,
};
use steam_vent::{Connection, ConnectionError, EResult, LoginError, ServerList};

/// What reconnecting from the saved login came to.
pub enum SavedOutcome {
    /// Logged in; the connection is ready to use.
    Connected(Box<Connection>, SavedLogin),
    /// Nothing is saved.
    NoSavedLogin,
    /// The saved token is past (or about to pass) its expiry. Removed.
    Expired(SavedLogin),
    /// Steam refused the saved token (revoked or invalidated). Removed.
    Rejected(SavedLogin),
}

/// True when a failed *token* logon means the token itself is dead, as
/// opposed to a network problem or rate limit that a retry could clear.
///
/// Steam answers an expired or revoked token with `InvalidPassword`, which
/// steam-vent reports as `InvalidCredentials`; it is not a wrong-password
/// error here because no password was sent.
pub fn is_dead_credential(error: &ConnectionError) -> bool {
    match error {
        ConnectionError::AccessToken(_) => true,
        ConnectionError::LoginError(LoginError::InvalidCredentials) => true,
        ConnectionError::LoginError(LoginError::Unknown(result)) => {
            matches!(
                result,
                // InvalidName is what Steam answers for a token that does not
                // belong to the account it is presented for (found by sending
                // a forged token to the live servers).
                EResult::AccessDenied | EResult::Expired | EResult::Revoked | EResult::InvalidName
            )
        }
        _ => false,
    }
}

/// Reconnect using the saved refresh token. A dead token is deleted so the
/// next attempt starts clean; any other failure leaves it in place.
pub async fn connect_saved(store: &LoginStore) -> Result<SavedOutcome> {
    let Some(saved) = store.load_login()? else {
        return Ok(SavedOutcome::NoSavedLogin);
    };
    if !saved.is_usable_at(now_secs()) {
        store.clear_login()?;
        return Ok(SavedOutcome::Expired(saved));
    }

    let servers = ServerList::discover()
        .await
        .context("Could not find Steam servers")?;
    match Connection::access(&servers, &saved.account, &saved.refresh_token).await {
        Ok(connection) => Ok(SavedOutcome::Connected(Box::new(connection), saved)),
        Err(error) if is_dead_credential(&error) => {
            store.clear_login()?;
            Ok(SavedOutcome::Rejected(saved))
        }
        Err(error) => Err(anyhow!(error).context(
            "Could not reconnect to Steam. If this keeps happening, run `modyssey steam login --force`",
        )),
    }
}

/// Log in with a password, asking for Steam Guard as needed, and save the
/// resulting refresh token. The password is not kept anywhere.
pub async fn login_with_password(
    store: &LoginStore,
    account: &str,
    password: &str,
) -> Result<(Connection, SavedLogin)> {
    let servers = ServerList::discover()
        .await
        .context("Could not find Steam servers")?;
    // Console code entry and "approve on your phone" run side by side; whichever
    // the account uses completes the login.
    let confirm = ConsoleAuthConfirmationHandler::default().or(DeviceConfirmationHandler);
    let connection = Connection::login(&servers, account, password, store.clone(), confirm)
        .await
        .map_err(|e| match e {
            ConnectionError::LoginError(LoginError::InvalidCredentials) => {
                anyhow!("Steam rejected the account name or password")
            }
            ConnectionError::LoginError(LoginError::RateLimited) => {
                anyhow!("Steam is rate limiting logins from this address; wait a while and retry")
            }
            other => anyhow!(other).context("Steam login failed"),
        })?;

    // For a password login steam-vent sends the new refresh token as the logon
    // token, so this is what a later `access` login needs.
    let Some(token) = connection.access_token() else {
        bail!("Steam did not return a login token, so nothing was saved");
    };
    let saved = SavedLogin::new(account, connection.steam_id().into(), token);
    store.save_login(&saved)?;
    Ok((connection, saved))
}

#[cfg(test)]
mod tests {
    use super::*;
    use steam_vent::NetworkError;

    #[test]
    fn dead_token_errors_clear_the_login_but_transient_ones_do_not() {
        let login = |e| ConnectionError::LoginError(e);
        // Steam's answer for an expired or revoked token.
        assert!(is_dead_credential(&login(LoginError::InvalidCredentials)));
        for result in [
            EResult::AccessDenied,
            EResult::Expired,
            EResult::Revoked,
            EResult::InvalidName,
        ] {
            assert!(
                is_dead_credential(&login(LoginError::Unknown(result))),
                "{result:?}"
            );
        }
        // Things a retry can fix must keep the saved login.
        assert!(!is_dead_credential(&login(LoginError::RateLimited)));
        assert!(!is_dead_credential(&login(LoginError::UnavailableAccount)));
        assert!(!is_dead_credential(&login(LoginError::Unknown(
            EResult::ServiceUnavailable
        ))));
        assert!(!is_dead_credential(&ConnectionError::Network(
            NetworkError::EOF
        )));
        assert!(!is_dead_credential(&ConnectionError::Aborted));
    }
}
