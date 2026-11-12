//! `clf3 steam`: log in to Steam once and keep the login.

use super::session::{connect_saved, login_with_password, SavedOutcome};
use super::store::{now_secs, LoginStore, SavedLogin};
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use std::io::{BufRead, Write};

#[derive(Debug, Args)]
pub struct SteamArgs {
    #[command(subcommand)]
    pub command: SteamCommand,
}

#[derive(Debug, Subcommand)]
pub enum SteamCommand {
    /// Log in to Steam and save the login so later runs need no password.
    Login {
        /// Steam account name (asked for if omitted).
        #[arg(long)]
        username: Option<String>,
        /// Log in again even if a usable saved login exists.
        #[arg(long)]
        force: bool,
    },
    /// Show the saved login.
    Status {
        /// Also reconnect to Steam to prove the saved login still works.
        #[arg(long)]
        check: bool,
    },
    /// Delete the saved login.
    Logout {
        /// Also forget the remembered-computer data, so the next login asks
        /// for a Steam Guard code again.
        #[arg(long)]
        forget_device: bool,
    },
}

fn describe(saved: &SavedLogin) -> String {
    match saved.days_left_at(now_secs()) {
        Some(days) => format!(
            "{} (SteamID {}, login valid for ~{days} more days)",
            saved.account, saved.steam_id
        ),
        None => format!("{} (SteamID {})", saved.account, saved.steam_id),
    }
}

fn prompt_line(label: &str) -> Result<String> {
    eprint!("{label}");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

pub async fn run(args: &SteamArgs) -> Result<()> {
    let store = LoginStore::default_location()?;
    match &args.command {
        SteamCommand::Login { username, force } => {
            if !*force {
                if let Some(saved) = store.load_login()? {
                    if saved.is_usable_at(now_secs()) {
                        println!("Already logged in as {}.", describe(&saved));
                        println!("Use `steam login --force` to log in again, or `steam logout`.");
                        return Ok(());
                    }
                }
            }
            let account = match username {
                Some(name) => name.clone(),
                None => prompt_line("Steam account name: ")?,
            };
            if account.is_empty() {
                bail!("No account name given");
            }
            // Read without echo from the terminal; never from arguments, which
            // land in shell history and the process list.
            let password = rpassword::prompt_password("Steam password: ")
                .context("Could not read the password")?;
            if password.is_empty() {
                bail!("No password given");
            }
            let (_connection, saved) = login_with_password(&store, &account, &password).await?;
            drop(password);
            println!("Logged in as {}.", describe(&saved));
            println!(
                "Login saved to {} (readable only by you).",
                store.login_path().display()
            );
        }
        SteamCommand::Status { check } => match store.load_login()? {
            None => println!("Not logged in. Run `ds2modpack steam login`."),
            Some(saved) => {
                println!("Saved login: {}", describe(&saved));
                if !saved.is_usable_at(now_secs()) {
                    println!("It has expired or is about to; run `ds2modpack steam login`.");
                } else if *check {
                    match connect_saved(&store).await? {
                        SavedOutcome::Connected(connection, _) => println!(
                            "Reconnected to Steam as SteamID {}.",
                            u64::from(connection.steam_id())
                        ),
                        SavedOutcome::Rejected(_) => {
                            println!("Steam no longer accepts this login (revoked?). It was removed; run `ds2modpack steam login`.")
                        }
                        SavedOutcome::Expired(_) => {
                            println!("The login had expired and was removed.")
                        }
                        SavedOutcome::NoSavedLogin => println!("Not logged in."),
                    }
                }
            }
        },
        SteamCommand::Logout { forget_device } => {
            // Read the account first: the remembered-computer data is per account.
            let account = store.load_login().ok().flatten().map(|l| l.account);
            let had = store.clear_login()?;
            if *forget_device {
                store.clear_guard(account.as_deref())?;
            }
            println!(
                "{}",
                if had {
                    "Saved Steam login deleted."
                } else {
                    "No saved Steam login."
                }
            );
            println!("Note: this only removes the copy on this computer; to revoke the login on Steam's side, use Steam > Settings > Security > Manage devices.");
        }
    }
    Ok(())
}
