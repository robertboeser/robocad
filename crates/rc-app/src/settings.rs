//! User settings stored as JSON in the OS configuration directory.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::app::ImportUnits;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// Mouse button mapping in the viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MousePreset {
    /// Right drag orbits, middle drag (or Shift + right) pans.
    #[default]
    Default,
    /// Middle drag orbits, Shift + middle pans (like Blender).
    Blender,
    /// Shift + middle orbits, middle pans (like Fusion 360).
    Fusion,
}

impl MousePreset {
    pub const ALL: [Self; 3] = [Self::Default, Self::Blender, Self::Fusion];

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "robocad (right = orbit, middle = pan)",
            Self::Blender => "Blender (middle = orbit, Shift+middle = pan)",
            Self::Fusion => "Fusion 360 (Shift+middle = orbit, middle = pan)",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    pub ui_scale: f32,
    /// Build plate for new projects.
    pub plate_size: [f64; 2],
    /// Grid step for new projects.
    pub grid_step: f64,
    pub mouse: MousePreset,
    pub import_units: ImportUnits,
    pub place_on_plate: bool,
    pub show_edges: bool,
    pub snapping: bool,
    /// Autosave interval in minutes (0 = off).
    pub autosave_minutes: u32,
    pub recent_files: Vec<PathBuf>,
    /// The first-steps card was dismissed.
    pub hints_shown: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            ui_scale: 1.0,
            plate_size: [220.0, 220.0],
            grid_step: 1.0,
            mouse: MousePreset::Default,
            import_units: ImportUnits::Millimetres,
            place_on_plate: true,
            show_edges: true,
            snapping: true,
            autosave_minutes: 2,
            recent_files: Vec::new(),
            hints_shown: false,
        }
    }
}

const MAX_RECENT: usize = 10;

impl Settings {
    /// Loads settings from `dir`, falling back to defaults on any problem.
    pub fn load(dir: &Path) -> Self {
        std::fs::read_to_string(dir.join("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Saves settings atomically; errors are logged, not fatal.
    pub fn save(&self, dir: &Path) {
        let result = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(dir)?;
            let tmp = dir.join("settings.json.tmp");
            std::fs::write(
                &tmp,
                serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?,
            )?;
            std::fs::rename(tmp, dir.join("settings.json"))
        })();
        if let Err(e) = result {
            log::warn!("could not save settings: {e}");
        }
    }

    pub fn add_recent(&mut self, path: &Path) {
        self.recent_files.retain(|p| p != path);
        self.recent_files.insert(0, path.to_path_buf());
        self.recent_files.truncate(MAX_RECENT);
    }
}

/// Where settings and autosaves live.
#[derive(Clone, Debug)]
pub struct AppDirs {
    pub config: PathBuf,
    pub data: PathBuf,
}

impl AppDirs {
    /// The OS default locations (e.g. `~/Library/Application Support/org.robocad.robocad`).
    pub fn system() -> Self {
        if let Some(d) = directories::ProjectDirs::from("org", "robocad", "robocad") {
            Self {
                config: d.config_dir().to_path_buf(),
                data: d.data_dir().to_path_buf(),
            }
        } else {
            let base = std::env::temp_dir().join("robocad");
            Self {
                config: base.clone(),
                data: base,
            }
        }
    }

    pub fn autosave_file(&self) -> PathBuf {
        self.data.join("autosave.rcad")
    }

    /// Side file remembering which project the autosave belongs to.
    pub fn autosave_origin(&self) -> PathBuf {
        self.data.join("autosave.origin")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_recent() {
        let dir =
            std::env::temp_dir().join(format!("robocad-settings-test-{}", std::process::id()));
        let mut s = Settings {
            ui_scale: 1.25,
            ..Settings::default()
        };
        for i in 0..12 {
            s.add_recent(Path::new(&format!("/tmp/{i}.rcad")));
        }
        s.add_recent(Path::new("/tmp/3.rcad"));
        assert_eq!(s.recent_files.len(), MAX_RECENT);
        assert_eq!(s.recent_files[0], Path::new("/tmp/3.rcad"));
        s.save(&dir);
        assert_eq!(Settings::load(&dir), s);
        // Unknown or missing fields fall back to defaults.
        std::fs::write(
            dir.join("settings.json"),
            r#"{"ui_scale": 2.0, "future": 1}"#,
        )
        .unwrap();
        let l = Settings::load(&dir);
        assert_eq!(l.ui_scale, 2.0);
        assert_eq!(l.grid_step, 1.0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
