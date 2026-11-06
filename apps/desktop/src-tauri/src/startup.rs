use crate::service::CommandError;
use serde::Serialize;
#[cfg(not(windows))]
use tauri_plugin_autostart::ManagerExt;

pub const LOGIN_ARGUMENT: &str = "--autostart";
#[cfg(debug_assertions)]
pub const ENTRY_NAME: &str = "RusTor Development";
#[cfg(not(debug_assertions))]
pub const ENTRY_NAME: &str = "RusTor";

pub trait Registration: Send + Sync {
    fn enabled(&self) -> Result<bool, CommandError>;
    fn set_enabled(&self, enabled: bool) -> Result<(), CommandError>;
}

pub struct NativeRegistration {
    #[cfg(not(windows))]
    app: tauri::AppHandle,
}
impl NativeRegistration {
    pub fn new(_app: tauri::AppHandle) -> Self {
        Self {
            #[cfg(not(windows))]
            app: _app,
        }
    }
}
impl Registration for NativeRegistration {
    fn enabled(&self) -> Result<bool, CommandError> {
        #[cfg(windows)]
        return windows::enabled();
        #[cfg(not(windows))]
        self.app
            .autolaunch()
            .is_enabled()
            .map_err(|error| startup_error(error.to_string()))
    }
    fn set_enabled(&self, enabled: bool) -> Result<(), CommandError> {
        #[cfg(windows)]
        let result = windows::set_enabled(enabled);
        #[cfg(not(windows))]
        let result = {
            let manager = self.app.autolaunch();
            if enabled {
                manager.enable()
            } else {
                manager.disable()
            }
            .map_err(|error| startup_error(error.to_string()))
        };
        result?;
        if self.enabled()? != enabled {
            return Err(startup_error(
                "The operating system did not confirm the change.",
            ));
        }
        Ok(())
    }
}

// auto-launch 0.6 writes an unquoted Windows path. Use a per-user entry with a
// quoted path instead; never write HKLM or request administrator privileges.
#[cfg(windows)]
mod windows {
    use super::*;
    use windows_registry::{CURRENT_USER, Type};
    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const APPROVED: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
    pub fn enabled() -> Result<bool, CommandError> {
        let registered = match CURRENT_USER
            .open(RUN)
            .and_then(|key| key.get_string(ENTRY_NAME))
        {
            Ok(_) => true,
            Err(error) if error.code().0 as u32 == 0x80070002 => false,
            Err(error) => return Err(startup_error(error.to_string())),
        };
        if !registered {
            return Ok(false);
        }
        match CURRENT_USER
            .open(APPROVED)
            .and_then(|key| key.get_value(ENTRY_NAME))
        {
            Ok(bytes) => Ok(bytes.first().is_none_or(|value| value & 1 == 0)),
            Err(error) if error.code().0 as u32 == 0x80070002 => Ok(true),
            Err(error) => Err(startup_error(error.to_string())),
        }
    }
    pub fn set_enabled(enabled: bool) -> Result<(), CommandError> {
        let result = if enabled {
            let exe = std::env::current_exe().map_err(|error| startup_error(error.to_string()))?;
            let command = format!("\"{}\" {LOGIN_ARGUMENT}", exe.display());
            CURRENT_USER
                .create(RUN)
                .and_then(|key| key.set_string(ENTRY_NAME, command))
                .and_then(|_| match CURRENT_USER.options().write().open(APPROVED) {
                    Ok(key) => key.set_bytes(
                        ENTRY_NAME,
                        Type::Bytes,
                        &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                    ),
                    Err(error) if error.code().0 as u32 == 0x80070002 => Ok(()),
                    Err(error) => Err(error),
                })
        } else {
            match CURRENT_USER
                .options()
                .write()
                .open(RUN)
                .and_then(|key| key.remove_value(ENTRY_NAME))
            {
                Err(error) if error.code().0 as u32 == 0x80070002 => Ok(()),
                result => result,
            }
        };
        result.map_err(|error| startup_error(error.to_string()))
    }
}

#[cfg(test)]
pub struct UnavailableRegistration;
#[cfg(test)]
impl Registration for UnavailableRegistration {
    fn enabled(&self) -> Result<bool, CommandError> {
        Err(startup_error("Login registration is unavailable."))
    }
    fn set_enabled(&self, _: bool) -> Result<(), CommandError> {
        self.enabled().map(|_| ())
    }
}

#[derive(Clone, Serialize)]
pub struct Status {
    pub enabled: Option<bool>,
    pub error: Option<CommandError>,
}

pub fn status(registration: &dyn Registration) -> Status {
    match registration.enabled() {
        Ok(enabled) => Status {
            enabled: Some(enabled),
            error: None,
        },
        Err(error) => Status {
            enabled: None,
            error: Some(error),
        },
    }
}

// Change the OS registration first, then persist settings. If persistence fails,
// restore the actual previous OS value, not a possibly stale stored preference.
pub fn apply(
    registration: &dyn Registration,
    desired: bool,
    saved: bool,
) -> Result<Option<bool>, CommandError> {
    let previous = match registration.enabled() {
        Ok(previous) => previous,
        Err(_) if desired == saved => return Ok(None),
        Err(error) => return Err(error),
    };
    if previous == desired {
        return Ok(None);
    }
    if let Err(error) = registration.set_enabled(desired) {
        return Err(rollback(registration, Some(previous), error));
    }
    Ok(Some(previous))
}

pub fn rollback(
    registration: &dyn Registration,
    previous: Option<bool>,
    error: CommandError,
) -> CommandError {
    if let Some(previous) = previous
        && let Err(rollback_error) = registration.set_enabled(previous)
    {
        return CommandError::new(
            "startup_rollback_failed",
            format!(
                "{} Login registration could not be restored: {}. Reopen settings to check its actual state.",
                error.message, rollback_error.message
            ),
        );
    }
    error
}

fn startup_error(detail: impl AsRef<str>) -> CommandError {
    CommandError::new(
        "startup_unavailable",
        format!(
            "Start at login could not be read or changed. {}",
            detail.as_ref()
        ),
    )
}

pub fn is_login_launch(args: &[String]) -> bool {
    args.iter().any(|arg| arg == LOGIN_ARGUMENT)
}

pub fn should_minimize(
    login_launch: bool,
    enabled: bool,
    preference: bool,
    tray_available: bool,
    foreground_requested: bool,
    ready: bool,
) -> bool {
    login_launch && enabled && preference && tray_available && !foreground_requested && ready
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    struct Fake {
        value: Mutex<bool>,
        calls: Mutex<Vec<bool>>,
        fail: bool,
    }
    impl Registration for Fake {
        fn enabled(&self) -> Result<bool, CommandError> {
            Ok(*self.value.lock().unwrap())
        }
        fn set_enabled(&self, enabled: bool) -> Result<(), CommandError> {
            self.calls.lock().unwrap().push(enabled);
            if self.fail {
                return Err(startup_error("test failure"));
            }
            *self.value.lock().unwrap() = enabled;
            Ok(())
        }
    }
    #[test]
    fn startup_changes_and_rollbacks_follow_actual_registration() {
        let registration = Fake {
            value: Mutex::new(false),
            calls: Mutex::new(vec![]),
            fail: false,
        };
        let previous = apply(&registration, true, true).unwrap();
        assert_eq!(previous, Some(false));
        assert_eq!(status(&registration).enabled, Some(true));
        let error = rollback(
            &registration,
            previous,
            CommandError::new("settings_not_saved", "save failed"),
        );
        assert_eq!(error.code, "settings_not_saved");
        assert!(!registration.enabled().unwrap());
        assert_eq!(*registration.calls.lock().unwrap(), vec![true, false]);
        assert_eq!(apply(&registration, false, true).unwrap(), None);
    }
    #[test]
    fn registration_errors_do_not_claim_success_or_block_unrelated_settings() {
        let unavailable = UnavailableRegistration;
        assert_eq!(status(&unavailable).enabled, None);
        assert!(status(&unavailable).error.is_some());
        assert!(apply(&unavailable, true, false).is_err());
        assert_eq!(apply(&unavailable, false, false).unwrap(), None);
        let failing = Fake {
            value: Mutex::new(false),
            calls: Mutex::new(vec![]),
            fail: true,
        };
        assert_eq!(
            apply(&failing, true, false).unwrap_err().code,
            "startup_rollback_failed"
        );
        assert!(!failing.enabled().unwrap());
    }
    #[test]
    fn only_ready_login_launches_with_a_tray_can_start_minimized() {
        assert!(is_login_launch(&["app".into(), "--autostart".into()]));
        assert!(!is_login_launch(&[
            "app".into(),
            "--autostart=false".into()
        ]));
        assert!(should_minimize(true, true, true, true, false, true));
        for flags in [
            [false, true, true, true, false, true],
            [true, false, true, true, false, true],
            [true, true, false, true, false, true],
            [true, true, true, false, false, true],
            [true, true, true, true, true, true],
            [true, true, true, true, false, false],
        ] {
            assert!(!should_minimize(
                flags[0], flags[1], flags[2], flags[3], flags[4], flags[5]
            ));
        }
    }
}
