//! Whether the language server may run content mappers.
//!
//! A mapper is code from `node_modules` that the server starts as a child
//! process, so it is off until the user says it is trusted. Two inputs:
//!
//! - `initializationOptions.runExternalCode`, which the editor sends (and
//!   which an editor can turn off for an untrusted workspace);
//! - `tsc --lsp --stdio --runExternalCode`, for editors that start a server
//!   from a command line and have no initialization options (Zed with a
//!   `binary.arguments` entry, Neovim `cmd`, Helix `args`).
//!
//! The flag is the default, not an override: an explicit option from the
//! client wins, so a client that withholds trust still withholds it.
//! Nothing here trusts a mapper because it sits in `node_modules`.

use std::sync::atomic::{AtomicBool, Ordering};

static FLAG: AtomicBool = AtomicBool::new(false);

/// Record `tsc --lsp ... --runExternalCode`.
pub fn set_command_line_flag(on: bool) {
    FLAG.store(on, Ordering::Relaxed);
}

/// The effective setting for a server whose client sent `option`.
pub fn run_external_code(option: Option<bool>) -> bool {
    resolve(option, FLAG.load(Ordering::Relaxed))
}

fn resolve(option: Option<bool>, flag: bool) -> bool {
    option.unwrap_or(flag)
}

#[cfg(test)]
mod tests {
    use super::resolve;

    #[test]
    fn off_unless_the_user_enabled_it() {
        assert!(!resolve(None, false));
    }

    #[test]
    fn the_flag_is_the_default_without_a_client_option() {
        assert!(resolve(None, true));
    }

    #[test]
    fn a_client_option_wins_over_the_flag() {
        assert!(!resolve(Some(false), true));
        assert!(resolve(Some(true), false));
    }
}
