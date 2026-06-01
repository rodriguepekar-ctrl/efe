//! Interactive prompts : confirm, input, select.
//!
//! All prompts honour the `ConfirmMode` from `AdminConfig` :
//!   - `Interactive` (default) : ask the user, fail if no TTY.
//!   - `AlwaysYes`              : auto-accept (= `--yes`).
//!   - `AlwaysNo`               : auto-reject (used in CI dry-runs).

use std::io::IsTerminal;

use colored::Colorize;
use dialoguer::theme::ColorfulTheme;

use crate::config::ConfirmMode;
use crate::error::AdminError;

/// Prompt the user for Y/N confirmation. Honours `ConfirmMode`.
pub fn confirm(prompt: &str, mode: ConfirmMode) -> Result<bool, AdminError> {
    match mode {
        ConfirmMode::AlwaysYes => Ok(true),
        ConfirmMode::AlwaysNo => Ok(false),
        ConfirmMode::Interactive => {
            if !std::io::stdin().is_terminal() {
                return Err(AdminError::invalid(
                    "destructive operation requires --yes or a TTY for confirmation",
                ));
            }
            let theme = ColorfulTheme::default();
            let res = dialoguer::Confirm::with_theme(&theme)
                .with_prompt(format!("{} {}", "[?]".yellow().bold(), prompt))
                .default(false)
                .interact()
                .map_err(|e| AdminError::Internal(format!("dialoguer: {e}")))?;
            Ok(res)
        }
    }
}

/// Confirm a destructive operation by re-typing the resource name.
/// Used for `tenant delete`, `key revoke` etc.
pub fn confirm_typed(resource_name: &str, mode: ConfirmMode) -> Result<bool, AdminError> {
    if !matches!(mode, ConfirmMode::Interactive) {
        return confirm(&format!("destructive op on '{resource_name}'"), mode);
    }
    if !std::io::stdin().is_terminal() {
        return Err(AdminError::invalid(
            "destructive operation requires --yes or a TTY for confirmation",
        ));
    }
    let theme = ColorfulTheme::default();
    let typed: String = dialoguer::Input::with_theme(&theme)
        .with_prompt(format!("Re-type '{resource_name}' to confirm"))
        .interact_text()
        .map_err(|e| AdminError::Internal(format!("dialoguer: {e}")))?;
    Ok(typed.trim() == resource_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_yes_is_true() {
        assert!(confirm("ok?", ConfirmMode::AlwaysYes).unwrap());
    }

    #[test]
    fn always_no_is_false() {
        assert!(!confirm("ok?", ConfirmMode::AlwaysNo).unwrap());
    }
}
