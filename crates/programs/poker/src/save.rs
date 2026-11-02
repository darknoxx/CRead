//! The player's wallet, kept next to CRead's other preferences.
//!
//! This is deliberately not a vault document: a bankroll is machine state, and
//! WebDAV has no business syncing it between two different players.

use std::path::{Path, PathBuf};

use cread_core::storage;

/// The first bankroll: ten dollars.
pub const START_BALANCE: u64 = 1_000;
/// The smallest bet the machine accepts.
pub const MIN_BET: u64 = 10;

/// What survives between sessions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wallet {
    /// Current bankroll, in cents.
    pub balance: u64,
    /// Highest bankroll ever reached, in cents.
    pub best: u64,
    /// How often the machine went broke. Permanent, like a highscore.
    pub bankrupts: u64,
}

impl Default for Wallet {
    fn default() -> Self {
        Self {
            balance: START_BALANCE,
            best: START_BALANCE,
            bankrupts: 0,
        }
    }
}

/// The save file, beside `prefs.json`.
pub fn path() -> PathBuf {
    storage::config_dir().join("poker.save")
}

/// Read the wallet. A missing or broken file is a fresh machine, never a crash.
pub fn load() -> Wallet {
    load_from(&path())
}

pub fn load_from(path: &Path) -> Wallet {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Wallet::default();
    };
    let mut wallet = Wallet {
        balance: START_BALANCE,
        best: START_BALANCE,
        bankrupts: 0,
    };
    let mut saw_balance = false;
    let mut saw_best = false;
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().parse::<u64>().ok();
        match key.trim() {
            "balance" => {
                if let Some(value) = value {
                    wallet.balance = value;
                    saw_balance = true;
                }
            }
            "best" => {
                if let Some(value) = value {
                    wallet.best = value;
                    saw_best = true;
                }
            }
            "bankrupts" => {
                if let Some(value) = value {
                    wallet.bankrupts = value;
                }
            }
            _ => {}
        }
    }
    if !saw_balance {
        wallet.balance = START_BALANCE;
    }
    if !saw_best {
        wallet.best = wallet.balance.max(START_BALANCE);
    }
    wallet.best = wallet.best.max(wallet.balance);
    wallet
}

pub fn save_to(path: &Path, wallet: &Wallet) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = format!(
        "balance={}\nbest={}\nbankrupts={}\n",
        wallet.balance, wallet.best, wallet.bankrupts
    );
    storage::save_atomic(path, &text)
}

/// `1234` cents as `$12.34`.
pub fn money(cents: u64) -> String {
    format!("${}.{:02}", cents / 100, cents % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("cread-poker-save-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("poker.save")
    }

    #[test]
    fn a_missing_file_is_ten_dollars() {
        let path = temp("missing");
        assert_eq!(load_from(&path), Wallet::default());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn wallet_survives_a_round_trip() {
        let path = temp("round");
        let wallet = Wallet {
            balance: 2_345,
            best: 9_999,
            bankrupts: 7,
        };
        save_to(&path, &wallet).unwrap();
        assert_eq!(load_from(&path), wallet);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_broken_file_falls_back_to_the_start() {
        let path = temp("broken");
        std::fs::write(&path, "not a wallet\nbalance=nope\n").unwrap();
        assert_eq!(load_from(&path), Wallet::default());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn best_is_never_below_the_balance() {
        let path = temp("best");
        std::fs::write(&path, "balance=4200\nbest=100\n").unwrap();
        let wallet = load_from(&path);
        assert_eq!(wallet.balance, 4_200);
        assert_eq!(wallet.best, 4_200);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_old_wallet_gets_zero_bankrupts() {
        let path = temp("old-wallet");
        std::fs::write(&path, "balance=1500\nbest=2500\n").unwrap();
        let wallet = load_from(&path);
        assert_eq!(wallet.balance, 1_500);
        assert_eq!(wallet.best, 2_500);
        assert_eq!(wallet.bankrupts, 0);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn money_shows_cents() {
        assert_eq!(money(0), "$0.00");
        assert_eq!(money(10), "$0.10");
        assert_eq!(money(1_000), "$10.00");
        assert_eq!(money(14_000), "$140.00");
    }
}
