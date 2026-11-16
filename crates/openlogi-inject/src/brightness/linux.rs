//! Linux's standard backlight ABI, with writes mediated by the active logind session.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{Adjustment, BrightnessError, ParameterValue, feedback, next_level};

struct Backlight {
    path: PathBuf,
    maximum: u32,
}

impl Backlight {
    fn discover(root: &Path) -> Result<Self, BrightnessError> {
        let mut candidates = Vec::new();
        for entry in std::fs::read_dir(root)? {
            let path = entry?.path();
            let Ok(maximum) = number(&path.join("max_brightness")) else {
                continue;
            };
            if maximum == 0 {
                continue;
            }
            let kind = std::fs::read_to_string(path.join("type")).unwrap_or_default();
            let priority = match kind.trim() {
                "firmware" => 0,
                "platform" => 1,
                "raw" => 2,
                _ => 3,
            };
            candidates.push((priority, path, maximum));
        }
        candidates.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));
        candidates
            .into_iter()
            .next()
            .map(|(_, path, maximum)| Self { path, maximum })
            .ok_or(BrightnessError::Unavailable)
    }

    fn read(&self) -> Result<ParameterValue, BrightnessError> {
        let actual = self.path.join("actual_brightness");
        let current = if actual.exists() {
            number(&actual)?
        } else {
            number(&self.path.join("brightness"))?
        };
        feedback(current, self.maximum)
    }

    fn adjust(
        &self,
        direction: Adjustment,
        write: impl FnOnce(u32) -> Result<(), BrightnessError>,
    ) -> Result<ParameterValue, BrightnessError> {
        let current = number(&self.path.join("brightness"))?;
        write(next_level(current, self.maximum, direction)?)?;
        self.read()
    }

    fn set(&self, desired: u32) -> Result<(), BrightnessError> {
        let connection = zbus::blocking::connection::Builder::system()
            .map_err(BrightnessError::from)?
            .method_timeout(Duration::from_secs(2))
            .build()
            .map_err(BrightnessError::from)?;
        let session = match std::env::var("XDG_SESSION_ID")
            .ok()
            .filter(|id| !id.is_empty())
        {
            Some(id) => {
                let manager = zbus::blocking::Proxy::new(
                    &connection,
                    "org.freedesktop.login1",
                    "/org/freedesktop/login1",
                    "org.freedesktop.login1.Manager",
                )
                .map_err(BrightnessError::from)?;
                manager
                    .call::<_, _, zbus::zvariant::OwnedObjectPath>("GetSession", &(id,))
                    .map_err(BrightnessError::from)?
            }
            None => {
                zbus::zvariant::OwnedObjectPath::try_from("/org/freedesktop/login1/session/auto")
                    .map_err(|error| BrightnessError::Backend(error.to_string()))?
            }
        };
        let proxy = zbus::blocking::Proxy::new(
            &connection,
            "org.freedesktop.login1",
            session,
            "org.freedesktop.login1.Session",
        )
        .map_err(BrightnessError::from)?;
        let name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(BrightnessError::InvalidValue)?;
        proxy
            .call::<_, _, ()>("SetBrightness", &("backlight", name, desired))
            .map_err(BrightnessError::from)
    }
}

impl From<zbus::Error> for BrightnessError {
    fn from(error: zbus::Error) -> Self {
        Self::Backend(error.to_string())
    }
}

fn number(path: &Path) -> Result<u32, BrightnessError> {
    std::fs::read_to_string(path)?
        .trim()
        .parse()
        .map_err(|_| BrightnessError::InvalidValue)
}

pub(super) fn read() -> Result<ParameterValue, BrightnessError> {
    Backlight::discover(Path::new("/sys/class/backlight"))?.read()
}

pub(super) fn adjust(direction: Adjustment) -> Result<ParameterValue, BrightnessError> {
    let control = Backlight::discover(Path::new("/sys/class/backlight"))?;
    control.adjust(direction, |desired| control.set(desired))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires an active logind session and a real system backlight"]
    fn native_backlight_read_write_same_value_round_trip() {
        let control = Backlight::discover(Path::new("/sys/class/backlight")).unwrap();
        let current = number(&control.path.join("brightness")).unwrap();
        let before = control.read().unwrap();
        control.set(current).unwrap();
        let actual = control.read().unwrap();
        println!(
            "BACKLIGHT PASS: logind accepted the current native level, before={}, readback={}",
            before.text(),
            actual.text()
        );
    }

    #[test]
    fn native_backlight_selection_and_live_values_follow_the_standard_abi() {
        let dir = tempfile::tempdir().unwrap();
        for (name, kind, value) in [("raw", "raw", 800), ("firmware", "firmware", 250)] {
            let path = dir.path().join(name);
            std::fs::create_dir(&path).unwrap();
            for (file, value) in [
                ("type", kind.to_owned()),
                ("max_brightness", "1000".into()),
                ("brightness", "500".into()),
                ("actual_brightness", value.to_string()),
            ] {
                std::fs::write(path.join(file), value).unwrap();
            }
        }
        let backlight = Backlight::discover(dir.path()).unwrap();
        assert_eq!(backlight.path.file_name().unwrap(), "firmware");
        assert_eq!(backlight.read().unwrap().text(), "25%");
        std::fs::write(backlight.path.join("actual_brightness"), "750").unwrap();
        assert_eq!(backlight.read().unwrap().text(), "75%");
        std::fs::write(backlight.path.join("actual_brightness"), "invalid").unwrap();
        backlight.read().unwrap_err();
    }

    #[test]
    fn writes_read_back_the_driver_and_propagate_rejections() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("brightness"), "500").unwrap();
        std::fs::write(dir.path().join("actual_brightness"), "480").unwrap();
        let control = Backlight {
            path: dir.path().to_owned(),
            maximum: 1000,
        };
        let result = control
            .adjust(Adjustment::Increase, |desired| {
                assert_eq!(desired, 550);
                // Drivers may quantize the requested value. Feedback stays measured.
                std::fs::write(dir.path().join("actual_brightness"), "540")?;
                Ok(())
            })
            .unwrap();
        assert_eq!(result.text(), "54%");
        let error = control
            .adjust(Adjustment::Decrease, |_| {
                Err(BrightnessError::Backend("denied".into()))
            })
            .unwrap_err();
        assert!(matches!(error, BrightnessError::Backend(_)));
        assert_eq!(control.read().unwrap().text(), "54%");
    }
}
