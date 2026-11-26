//! The config file (DMN-009): `[daemon]` holds the `omfp daemon` options (merged in [`crate::config::Config`]),
//! `[ui]` and `[keymap]` the UI settings (UI-097) the daemon serves with `settings.get` and writes back with
//! `settings.set`, keeping comments and the other keys ([`write_back`]). `[daemon] delete_archived_after`
//! ([`DeleteArchivedAfter`], DMN-395) is the one `[daemon]` key served and written back too, applied live (DMN-396).
//!
//! ```toml
//! [daemon]
//! bind = "127.0.0.1:7420"
//! title_generation = false
//! # Origins besides the daemon's own allowed to open the WebSocket (DMN-140): the UI's URL behind a tunnel or a
//! # reverse proxy, exactly `http(s)://host[:port]`; no `*`.
//! allowed_origins = ["https://omfp.example.ts.net"]
//! delete_archived_after = "7d"  # <n>m | <n>h | <n>d | <n>w | never (also 0)
//!
//! [ui]
//! density = "normal"            # compact | normal
//! font_size = 17                # root font size in px, 10..=28
//! motion = "full"               # full | reduced | none
//! notifications = "background"  # background | always | never
//! theme = "catppuccin"          # synth | catppuccin | gruvbox | tokyo-night | nord | solarized | rose-pine | graphite
//!                               # | paper | clay | primer | accent | matugen (the user theme, theme.css)
//! show_xdev_notices = false     # show informational MCP mount/unmount notices in the transcript
//! skin = "crush"                # crush | boring
//! recent_threads = 20           # hidden threads a window keeps followed, 0..=100 or "all"
//! live_voice = "sol"            # arbor | breeze | cove | ember | juniper | maple | sol | spruce | vale (omp's voices)
//!
//! [ui.fonts]
//! prose = "Literata, serif"
//!
//! [keymap]                      # normal mode: Threads and View mode both
//! "Space t" = "window.theme"
//! "g g" = ""                    # removes the default binding
//!
//! [keymap.threads]              # also [keymap.view], [keymap.insert], [keymap.cmdline]
//! "Ctrl-j" = "thread.next"
//! ```

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use omfp_protocol::{
    AllThreads, BuiltinTheme, DaemonSettings, Density, FontSettings, KeymapEntry, KeymapMode,
    LiveVoice, Motion, NotificationMode, RecentThreads, SetSettingsParams, Settings, Skin,
    UiSettings,
};
use serde::Deserialize;
use toml_edit::{DocumentMut, Item, Table};

use crate::wire_name;

/// The `[daemon]` table: every `omfp daemon` option under the flag's name in snake case. Paths may start with `~`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DaemonOptions {
    pub bind: Option<SocketAddr>,
    pub data_dir: Option<PathBuf>,
    pub workspaces_root: Option<PathBuf>,
    pub omp_bin: Option<PathBuf>,
    pub static_dir: Option<PathBuf>,
    pub allow_omp_mismatch: Option<bool>,
    pub idle_timeout_minutes: Option<u64>,
    pub max_processes: Option<usize>,
    pub event_buffer: Option<usize>,
    /// `--allowed-origin`, plural as it is a list (DMN-140).
    pub allowed_origins: Option<Vec<String>>,
    pub theme_file: Option<PathBuf>,
    /// `false` is `--no-title-generation`.
    pub title_generation: Option<bool>,
    /// Applies without a restart, unlike the other keys (DMN-396).
    pub delete_archived_after: Option<DeleteArchivedAfter>,
}

impl DaemonOptions {
    /// The options with `~` in paths replaced by `home`.
    #[must_use]
    pub fn expanded(self, home: Option<&Path>) -> Self {
        let expand = |path: Option<PathBuf>| path.map(|path| expand_home(path, home));
        Self {
            data_dir: expand(self.data_dir),
            workspaces_root: expand(self.workspaces_root),
            omp_bin: expand(self.omp_bin),
            static_dir: expand(self.static_dir),
            theme_file: expand(self.theme_file),
            ..self
        }
    }
}

/// `~` and `~/…` start at `home`; other paths (and every path without a home) stay as they are.
#[must_use]
pub fn expand_home(path: PathBuf, home: Option<&Path>) -> PathBuf {
    let Some(home) = home else {
        return path;
    };
    match path.strip_prefix("~") {
        Ok(rest) => home.join(rest),
        Err(_) => path,
    }
}

/// `[daemon] delete_archived_after` and `--delete-archived-after` (DMN-395): how long a thread stays archived before
/// the daemon deletes it, kept as written (`7d` stays `7d`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteArchivedAfter {
    /// Archived threads stay: `never`, `0`.
    Never,
    /// `count` (at least 1) of `unit`: `m`, `h`, `d` or `w`.
    After { count: u32, unit: char },
}

/// The units of [`DeleteArchivedAfter`], in milliseconds.
const ARCHIVED_UNITS: [(char, i64); 4] = [
    ('m', 60_000),
    ('h', 3_600_000),
    ('d', 86_400_000),
    ('w', 604_800_000),
];

impl DeleteArchivedAfter {
    /// One week (DMN-395).
    pub const DEFAULT: Self = Self::After {
        count: 7,
        unit: 'd',
    };

    /// How long in milliseconds; `None` when archived threads stay.
    #[must_use]
    pub fn millis(self) -> Option<i64> {
        let Self::After { count, unit } = self else {
            return None;
        };
        ARCHIVED_UNITS
            .iter()
            .find(|(name, _)| *name == unit)
            .map(|(_, ms)| i64::from(count).saturating_mul(*ms))
    }
}

impl std::str::FromStr for DeleteArchivedAfter {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let invalid = || {
            format!(
                "`{text}` is no time to keep archived threads: use a number followed by m, h, d or w (minutes, hours, \
                 days, weeks; e.g. 7d), or never"
            )
        };
        if text == "never" || text == "0" {
            return Ok(Self::Never);
        }
        let Some(unit) = text.chars().next_back() else {
            return Err(invalid());
        };
        let digits = &text[..text.len() - unit.len_utf8()];
        if digits.is_empty()
            || !digits.bytes().all(|byte| byte.is_ascii_digit())
            || !ARCHIVED_UNITS.iter().any(|(name, _)| *name == unit)
        {
            return Err(invalid());
        }
        let count: u32 = digits.parse().map_err(|_| invalid())?;
        Ok(if count == 0 {
            Self::Never
        } else {
            Self::After { count, unit }
        })
    }
}

impl std::fmt::Display for DeleteArchivedAfter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Never => f.write_str("never"),
            Self::After { count, unit } => write!(f, "{count}{unit}"),
        }
    }
}

/// A string as [`DeleteArchivedAfter`] parses it, or the number `0`.
impl<'de> Deserialize<'de> for DeleteArchivedAfter {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Text(String),
            Number(i64),
        }
        match Raw::deserialize(deserializer)? {
            Raw::Text(text) => text.parse().map_err(serde::de::Error::custom),
            Raw::Number(0) => Ok(Self::Never),
            Raw::Number(number) => Err(serde::de::Error::custom(format!(
                "`{number}` needs a unit, e.g. \"{number}d\", or 0 to keep archived threads"
            ))),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    daemon: DaemonOptions,
    #[serde(default)]
    ui: RawUi,
    #[serde(default)]
    keymap: toml::Table,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawUi {
    density: Option<Density>,
    font_size: Option<u8>,
    motion: Option<Motion>,
    notifications: Option<NotificationMode>,
    fonts: Option<RawFonts>,
    theme: Option<BuiltinTheme>,
    show_xdev_notices: Option<bool>,
    skin: Option<Skin>,
    recent_threads: Option<RecentThreads>,
    live_voice: Option<LiveVoice>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFonts {
    prose: Option<String>,
    ui: Option<String>,
    code: Option<String>,
}

/// A parsed config file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigFile {
    pub daemon: DaemonOptions,
    pub ui: UiSettings,
    /// `[keymap]` (normal mode), then its mode tables (`threads`, `view`, `insert`, `cmdline`), each sorted by
    /// keys.
    pub keymap: Vec<KeymapEntry>,
}

impl ConfigFile {
    /// Parses the file's text; an unknown key or a wrong value is an error naming it.
    pub fn parse(text: &str) -> Result<Self, String> {
        let raw: RawFile = toml::from_str(text).map_err(|error| error.to_string())?;
        if let Some(size) = raw.ui.font_size
            && !FONT_SIZES.contains(&size)
        {
            return Err(format!(
                "`ui.font_size` must be between {} and {} (px), not {size}",
                FONT_SIZES.start(),
                FONT_SIZES.end()
            ));
        }
        if let Some(RecentThreads::Count(count)) = raw.ui.recent_threads
            && count > RECENT_THREADS_MAX
        {
            return Err(format!(
                "`ui.recent_threads` must be at most {RECENT_THREADS_MAX} or \"all\", not {count}"
            ));
        }
        let fonts = raw.ui.fonts.map(|fonts| FontSettings {
            prose: fonts.prose,
            ui: fonts.ui,
            code: fonts.code,
        });
        Ok(Self {
            daemon: raw.daemon,
            ui: UiSettings {
                density: raw.ui.density,
                font_size: raw.ui.font_size,
                motion: raw.ui.motion,
                notifications: raw.ui.notifications,
                fonts,
                theme: raw.ui.theme,
                show_xdev_notices: raw.ui.show_xdev_notices,
                skin: raw.ui.skin,
                recent_threads: raw.ui.recent_threads,
                live_voice: raw.ui.live_voice,
            },
            keymap: keymap(&raw.keymap)?,
        })
    }

    /// Reads and parses `path`; `None` when it does not exist. Errors name the file.
    pub fn read(path: &Path) -> Result<Option<Self>, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text)
                .map(Some)
                .map_err(|error| format!("{}: {error}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    }

    /// The settings the file holds, without error.
    #[must_use]
    pub fn settings(&self) -> Settings {
        Settings {
            daemon: self
                .daemon
                .delete_archived_after
                .map(|after| DaemonSettings {
                    delete_archived_after: Some(after.to_string()),
                }),
            ui: self.ui.clone(),
            keymap: self.keymap.clone(),
            error: None,
        }
    }
}

/// The accepted `[ui] font_size` values in px (UI-097).
pub const FONT_SIZES: std::ops::RangeInclusive<u8> = 10..=28;

/// The largest accepted `[ui] recent_threads` (UI-996): each kept thread's state stays in the window's memory.
pub const RECENT_THREADS_MAX: u8 = 100;

const MODE_TABLES: [(&str, KeymapMode); 4] = [
    ("threads", KeymapMode::Threads),
    ("view", KeymapMode::View),
    ("insert", KeymapMode::Insert),
    ("cmdline", KeymapMode::Cmdline),
];

/// The bindings of `[keymap]`: string values are normal-mode bindings (Threads and View mode both), the `threads`,
/// `view`, `insert` and `cmdline` sub-tables the other modes.
fn keymap(table: &toml::Table) -> Result<Vec<KeymapEntry>, String> {
    let mut entries = Vec::new();
    let mut modes = Vec::new();
    for (keys, value) in table {
        match value {
            toml::Value::String(command) => entries.push(KeymapEntry {
                mode: KeymapMode::Normal,
                keys: keys.clone(),
                command: command.clone(),
            }),
            toml::Value::Table(bindings) => {
                let Some((_, mode)) = MODE_TABLES.iter().find(|(name, _)| name == keys) else {
                    return Err(format!(
                        "unknown keymap table `keymap.{keys}`, expected `keymap.threads`, \
                         `keymap.view`, `keymap.insert` or `keymap.cmdline`"
                    ));
                };
                modes.push((*mode, keys, bindings));
            }
            _ => {
                return Err(format!("`keymap.\"{keys}\"` must be a command id string"));
            }
        }
    }
    for (mode, name, bindings) in modes {
        for (keys, value) in bindings {
            let Some(command) = value.as_str() else {
                return Err(format!(
                    "`keymap.{name}.\"{keys}\"` must be a command id string"
                ));
            };
            entries.push(KeymapEntry {
                mode,
                keys: keys.clone(),
                command: command.to_owned(),
            });
        }
    }
    Ok(entries)
}

/// `update` merged into the config file's `text` (empty when there is none): only the given values change; comments,
/// other keys and their order stay. The result is checked to parse.
pub fn write_back(text: &str, update: &SetSettingsParams) -> Result<String, String> {
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|error| format!("the config file is not valid TOML: {error}"))?;
    let delete_archived_after = update
        .daemon
        .as_ref()
        .and_then(|daemon| daemon.delete_archived_after.as_deref());
    if let Some(after) = delete_archived_after {
        let after: DeleteArchivedAfter = after
            .parse()
            .map_err(|error| format!("`daemon.delete_archived_after`: {error}"))?;
        set(
            child(doc.as_table_mut(), "daemon")?,
            "delete_archived_after",
            after.to_string(),
        );
    }
    if let Some(ui) = &update.ui {
        let table = child(doc.as_table_mut(), "ui")?;
        let recent_threads: Option<toml_edit::Value> =
            ui.recent_threads.map(|recent| match recent {
                RecentThreads::Count(count) => i64::from(count).into(),
                RecentThreads::All(AllThreads::All) => "all".into(),
            });
        for (key, value) in [
            ("density", ui.density.map(named)),
            ("font_size", ui.font_size.map(|size| i64::from(size).into())),
            ("motion", ui.motion.map(named)),
            ("notifications", ui.notifications.map(named)),
            ("theme", ui.theme.map(named)),
            (
                "show_xdev_notices",
                ui.show_xdev_notices.map(toml_edit::Value::from),
            ),
            ("skin", ui.skin.map(named)),
            ("live_voice", ui.live_voice.map(named)),
            ("recent_threads", recent_threads),
        ] {
            if let Some(value) = value {
                set(table, key, value);
            }
        }
        if let Some(fonts) = &ui.fonts {
            let table = child(table, "fonts")?;
            for (key, value) in [
                ("prose", &fonts.prose),
                ("ui", &fonts.ui),
                ("code", &fonts.code),
            ] {
                if let Some(value) = value {
                    set(table, key, value);
                }
            }
        }
    }
    for entry in update.keymap.iter().flatten() {
        if entry.keys.trim().is_empty() {
            return Err("a keymap entry needs keys".into());
        }
        let table = child(doc.as_table_mut(), "keymap")?;
        let table = match entry.mode {
            KeymapMode::Normal => table,
            KeymapMode::Threads => child(table, "threads")?,
            KeymapMode::View => child(table, "view")?,
            KeymapMode::Insert => child(table, "insert")?,
            KeymapMode::Cmdline => child(table, "cmdline")?,
        };
        set(table, &entry.keys, &entry.command);
    }
    let text = doc.to_string();
    ConfigFile::parse(&text)?;
    Ok(text)
}

/// The sub-table `key` of `parent`, created (as a `[header]` once it holds values) when missing.
fn child<'a>(parent: &'a mut Table, key: &str) -> Result<&'a mut Table, String> {
    parent
        .entry(key)
        .or_insert_with(|| {
            let mut table = Table::new();
            table.set_implicit(true);
            Item::Table(table)
        })
        .as_table_mut()
        .ok_or_else(|| format!("`{key}` in the config file is not a table"))
}

/// Sets `key` to `value`, keeping the comments around an existing value.
fn set(table: &mut Table, key: &str, value: impl Into<toml_edit::Value>) {
    let value = value.into();
    match table.get_mut(key).and_then(Item::as_value_mut) {
        Some(existing) => {
            let decor = existing.decor().clone();
            *existing = value;
            *existing.decor_mut() = decor;
        }
        None => table[key] = Item::Value(value),
    }
}

/// The config file's value of a `[ui]` enum: its protocol name.
fn named(value: impl serde::Serialize) -> toml_edit::Value {
    wire_name(value).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: UI-043, UI-097
    #[test]
    fn xdev_visibility_round_trips_without_changing_other_settings() {
        assert_eq!(ConfigFile::parse(FILE).unwrap().ui.show_xdev_notices, None);
        let mut text = FILE.to_owned();
        for visible in [true, false] {
            text = write_back(
                &text,
                &SetSettingsParams {
                    daemon: None,
                    ui: Some(UiSettings {
                        show_xdev_notices: Some(visible),
                        ..UiSettings::default()
                    }),
                    keymap: None,
                },
            )
            .unwrap();
            let mut expected = ConfigFile::parse(FILE).unwrap();
            expected.ui.show_xdev_notices = Some(visible);
            assert_eq!(ConfigFile::parse(&text).unwrap(), expected);
        }
        assert!(ConfigFile::parse("[ui]\nshow_xdev_notices = \"true\"").is_err());
    }

    // spec: UI-794, DMN-009
    #[test]
    fn the_skin_round_trips_without_changing_other_settings() {
        assert_eq!(ConfigFile::parse(FILE).unwrap().ui.skin, None);
        assert_eq!(
            ConfigFile::parse("[ui]\nskin = \"boring\"")
                .unwrap()
                .ui
                .skin,
            Some(Skin::Boring)
        );
        let mut text = FILE.to_owned();
        for skin in [Skin::Boring, Skin::Crush] {
            text = write_back(
                &text,
                &SetSettingsParams {
                    daemon: None,
                    ui: Some(UiSettings {
                        skin: Some(skin),
                        ..UiSettings::default()
                    }),
                    keymap: None,
                },
            )
            .unwrap();
            let mut expected = ConfigFile::parse(FILE).unwrap();
            expected.ui.skin = Some(skin);
            assert_eq!(ConfigFile::parse(&text).unwrap(), expected);
        }
        assert!(text.contains("skin = \"crush\""), "{text}");
    }

    // spec: UI-999, PRT-228, DMN-009
    #[test]
    fn every_live_voice_round_trips_by_omps_name_and_others_are_refused() {
        let mut text = FILE.to_owned();
        for voice in LiveVoice::ALL {
            text = write_back(
                &text,
                &SetSettingsParams {
                    daemon: None,
                    ui: Some(UiSettings {
                        live_voice: Some(voice),
                        ..UiSettings::default()
                    }),
                    keymap: None,
                },
            )
            .unwrap();
            assert!(
                text.contains(&format!("live_voice = \"{}\" # calmer", voice.as_str())),
                "{text}"
            );
            let mut expected = ConfigFile::parse(FILE).unwrap();
            expected.ui.live_voice = Some(voice);
            assert_eq!(ConfigFile::parse(&text).unwrap(), expected);
        }
        let error = ConfigFile::parse("[ui]\nlive_voice = \"alloy\"").unwrap_err();
        assert!(error.contains("alloy"), "{error}");
    }

    // spec: UI-996, DMN-009
    #[test]
    fn recent_threads_round_trips_and_is_bounded() {
        assert_eq!(ConfigFile::parse(FILE).unwrap().ui.recent_threads, None);
        let all = RecentThreads::All(AllThreads::All);
        for (value, written) in [
            (RecentThreads::Count(40), "recent_threads = 40\n"),
            (all, "recent_threads = \"all\"\n"),
        ] {
            let text = write_back(
                FILE,
                &SetSettingsParams {
                    daemon: None,
                    ui: Some(UiSettings {
                        recent_threads: Some(value),
                        ..UiSettings::default()
                    }),
                    keymap: None,
                },
            )
            .unwrap();
            assert!(text.contains(written), "{text}");
            let mut expected = ConfigFile::parse(FILE).unwrap();
            expected.ui.recent_threads = Some(value);
            assert_eq!(ConfigFile::parse(&text).unwrap(), expected);
        }
        let parsed = |text: &str| ConfigFile::parse(text).map(|file| file.ui.recent_threads);
        assert_eq!(
            parsed("[ui]\nrecent_threads = 0"),
            Ok(Some(RecentThreads::Count(0))),
            "no thread kept"
        );
        assert_eq!(parsed("[ui]\nrecent_threads = \"all\""), Ok(Some(all)));

        let too_many = SetSettingsParams {
            daemon: None,
            ui: Some(UiSettings {
                recent_threads: Some(RecentThreads::Count(RECENT_THREADS_MAX + 1)),
                ..UiSettings::default()
            }),
            keymap: None,
        };
        assert!(
            write_back("", &too_many)
                .unwrap_err()
                .contains("ui.recent_threads")
        );
        for wrong in ["-1", "\"some\"", "true"] {
            let error = parsed(&format!("[ui]\nrecent_threads = {wrong}")).unwrap_err();
            assert!(error.contains("recent_threads"), "{wrong}: {error}");
        }
    }

    const FILE: &str = r#"# My omfp.
[daemon]
max_processes = 4 # enough

[ui]
# Roomier rows.
density = "compact" # for now
font_size = 15
motion = "reduced"
theme = "gruvbox"
live_voice = "maple" # calmer
[ui.fonts]
prose = "Literata, serif"

[keymap]
"Space t" = "window.theme"
"g g" = ""

[keymap.insert]
"Ctrl-j" = "nav.next"
"#;

    fn entry(mode: KeymapMode, keys: &str, command: &str) -> KeymapEntry {
        KeymapEntry {
            mode,
            keys: keys.into(),
            command: command.into(),
        }
    }

    // spec: DMN-009, UI-097, UI-145, DMN-238, UI-999
    #[test]
    fn the_file_holds_daemon_options_ui_settings_and_keymap_entries() {
        let file = ConfigFile::parse(FILE).unwrap();

        assert_eq!(file.daemon.max_processes, Some(4));
        assert_eq!(
            file.ui,
            UiSettings {
                density: Some(Density::Compact),
                font_size: Some(15),
                motion: Some(Motion::Reduced),
                notifications: None,
                fonts: Some(FontSettings {
                    prose: Some("Literata, serif".into()),
                    ui: None,
                    code: None,
                }),
                theme: Some(BuiltinTheme::Gruvbox),
                show_xdev_notices: None,
                skin: None,
                recent_threads: None,
                live_voice: Some(LiveVoice::Maple),
            }
        );
        assert_eq!(
            file.keymap,
            [
                entry(KeymapMode::Normal, "Space t", "window.theme"),
                entry(KeymapMode::Normal, "g g", ""),
                entry(KeymapMode::Insert, "Ctrl-j", "nav.next"),
            ]
        );
        assert_eq!(ConfigFile::parse("").unwrap(), ConfigFile::default());
    }

    // spec: UI-097, UI-095
    #[test]
    fn the_threads_and_view_tables_bind_one_mode_each_and_are_written_back() {
        let text = r#"
[keymap]
j = "x"

[keymap.threads]
j = "thread.next"

[keymap.view]
j = "nav.next"
"#;
        let file = ConfigFile::parse(text).unwrap();
        assert_eq!(
            file.keymap,
            [
                entry(KeymapMode::Normal, "j", "x"),
                entry(KeymapMode::Threads, "j", "thread.next"),
                entry(KeymapMode::View, "j", "nav.next"),
            ]
        );

        let update = SetSettingsParams {
            ui: None,
            keymap: Some(vec![
                entry(KeymapMode::Threads, "g g", ""),
                entry(KeymapMode::View, "Ctrl-w x", "pane.right"),
            ]),
        };
        let written = ConfigFile::parse(&write_back("", &update).unwrap()).unwrap();
        assert_eq!(written.keymap, update.keymap.unwrap());
    }

    // spec: DMN-009, DMN-238
    #[test]
    fn unknown_keys_and_wrong_values_are_errors_naming_them() {
        for (text, named) in [
            ("[daemon]\nbnid = \"x\"", "bnid"),
            ("[colors]\na = 1", "colors"),
            ("[ui]\ndensty = \"normal\"", "densty"),
            ("[ui]\ndensity = \"huge\"", "huge"),
            ("[ui]\nmotion = \"slow\"", "slow"),
            ("[ui]\ntheme = \"dracula\"", "dracula"),
            ("[ui]\ntheme = \"tokyo_night\"", "tokyo_night"),
            ("[ui]\nskin = \"orca\"", "orca"),
            ("[ui]\nfont_size = 9", "ui.font_size"),
            ("[ui]\nfont_size = 29", "ui.font_size"),
            ("[ui]\nfont_size = \"big\"", "font_size"),
            ("[ui.fonts]\nserif = \"x\"", "serif"),
            ("[keymap.visual]\nj = \"nav.next\"", "keymap.visual"),
            ("[keymap]\nj = 1", "keymap.\"j\""),
            ("[keymap.insert]\nj = true", "keymap.insert.\"j\""),
            ("[daemon]\nbind = \"not an address\"", "bind"),
        ] {
            let error = ConfigFile::parse(text).unwrap_err();
            assert!(error.contains(named), "{text}: {error}");
        }
    }

    // spec: DMN-238, UI-510
    #[test]
    fn matugen_is_a_theme_read_and_written_back() {
        assert_eq!(
            ConfigFile::parse("[ui]\ntheme = \"matugen\"")
                .unwrap()
                .ui
                .theme,
            Some(BuiltinTheme::Matugen)
        );
        let text = write_back(
            FILE,
            &SetSettingsParams {
                daemon: None,
                ui: Some(UiSettings {
                    theme: Some(BuiltinTheme::Matugen),
                    ..UiSettings::default()
                }),
                keymap: None,
            },
        )
        .unwrap();
        assert!(text.contains("theme = \"matugen\"\n"), "{text}");
        let mut expected = ConfigFile::parse(FILE).unwrap();
        expected.ui.theme = Some(BuiltinTheme::Matugen);
        assert_eq!(ConfigFile::parse(&text).unwrap(), expected);
    }

    // spec: DMN-238, UI-510
    #[test]
    fn the_calm_palettes_are_themes_read_and_written_back() {
        for (theme, name) in [
            (BuiltinTheme::Graphite, "graphite"),
            (BuiltinTheme::Paper, "paper"),
            (BuiltinTheme::Clay, "clay"),
            (BuiltinTheme::Primer, "primer"),
        ] {
            let parsed = ConfigFile::parse(&format!("[ui]\ntheme = \"{name}\"")).unwrap();
            assert_eq!(parsed.ui.theme, Some(theme));
            let text = write_back(
                FILE,
                &SetSettingsParams {
                    daemon: None,
                    ui: Some(UiSettings {
                        theme: Some(theme),
                        ..UiSettings::default()
                    }),
                    keymap: None,
                },
            )
            .unwrap();
            assert!(text.contains(&format!("theme = \"{name}\"\n")), "{text}");
        }
    }

    // spec: DMN-009
    #[test]
    fn a_tilde_starts_at_home() {
        let home = Path::new("/home/u");
        assert_eq!(
            expand_home("~/data".into(), Some(home)),
            PathBuf::from("/home/u/data")
        );
        assert_eq!(
            expand_home("~".into(), Some(home)),
            PathBuf::from("/home/u")
        );
        assert_eq!(
            expand_home("/abs".into(), Some(home)),
            PathBuf::from("/abs")
        );
        assert_eq!(
            expand_home("~x/a".into(), Some(home)),
            PathBuf::from("~x/a")
        );
        assert_eq!(expand_home("~/data".into(), None), PathBuf::from("~/data"));
    }

    // spec: DMN-009, UI-097, UI-145, DMN-238, UI-999
    #[test]
    fn write_back_changes_only_the_given_values_and_keeps_comments() {
        let update = SetSettingsParams {
            daemon: None,
            ui: Some(UiSettings {
                density: Some(Density::Normal),
                font_size: Some(18),
                motion: Some(Motion::None),
                notifications: Some(NotificationMode::Never),
                fonts: Some(FontSettings {
                    prose: None,
                    ui: None,
                    code: Some("\"Iosevka\", monospace".into()),
                }),
                theme: Some(BuiltinTheme::TokyoNight),
                show_xdev_notices: None,
                skin: Some(Skin::Boring),
                live_voice: Some(LiveVoice::Vale),
                recent_threads: None,
            }),
            keymap: Some(vec![
                entry(KeymapMode::Normal, "Space t", "density.set"),
                entry(KeymapMode::Cmdline, "Ctrl-p", "nav.prev"),
            ]),
        };

        let text = write_back(FILE, &update).unwrap();

        assert_eq!(
            text,
            r#"# My omfp.
[daemon]
max_processes = 4 # enough

[ui]
# Roomier rows.
density = "normal" # for now
font_size = 18
motion = "none"
theme = "tokyo-night"
live_voice = "vale" # calmer
notifications = "never"
skin = "boring"
[ui.fonts]
prose = "Literata, serif"
code = '"Iosevka", monospace'

[keymap]
"Space t" = "density.set"
"g g" = ""

[keymap.insert]
"Ctrl-j" = "nav.next"

[keymap.cmdline]
Ctrl-p = "nav.prev"
"#
        );
    }

    // spec: DMN-009
    #[test]
    fn write_back_creates_the_tables_of_an_empty_file_and_refuses_broken_ones() {
        let update = SetSettingsParams {
            daemon: None,
            ui: Some(UiSettings {
                density: Some(Density::Compact),
                font_size: Some(20),
                ..UiSettings::default()
            }),
            keymap: Some(vec![entry(KeymapMode::Insert, "Ctrl-j", "")]),
        };
        let text = write_back("", &update).unwrap();
        let file = ConfigFile::parse(&text).unwrap();
        assert_eq!(file.ui.density, Some(Density::Compact));
        assert_eq!(file.ui.font_size, Some(20));
        assert_eq!(file.keymap, [entry(KeymapMode::Insert, "Ctrl-j", "")]);

        assert!(write_back("[ui", &update).is_err());
        assert!(write_back("ui = 1", &update).is_err());
        let too_big = SetSettingsParams {
            daemon: None,
            ui: Some(UiSettings {
                font_size: Some(40),
                ..UiSettings::default()
            }),
            keymap: None,
        };
        assert!(
            write_back("", &too_big)
                .unwrap_err()
                .contains("ui.font_size")
        );
        let no_keys = SetSettingsParams {
            daemon: None,
            ui: None,
            keymap: Some(vec![entry(KeymapMode::Normal, " ", "x")]),
        };
        assert!(write_back("", &no_keys).is_err());
    }

    fn archived(value: &str) -> Result<Option<DeleteArchivedAfter>, String> {
        ConfigFile::parse(&format!("[daemon]\ndelete_archived_after = {value}"))
            .map(|file| file.daemon.delete_archived_after)
    }

    // spec: DMN-395, DMN-009
    #[test]
    fn delete_archived_after_takes_minutes_to_weeks_or_never_and_refuses_the_rest() {
        assert_eq!(
            ConfigFile::parse(FILE)
                .unwrap()
                .daemon
                .delete_archived_after,
            None
        );
        assert_eq!(DeleteArchivedAfter::DEFAULT.millis(), Some(604_800_000));
        let after = |count, unit| Some(DeleteArchivedAfter::After { count, unit });
        for (value, parsed, millis) in [
            ("\"30m\"", after(30, 'm'), Some(1_800_000)),
            ("\"12h\"", after(12, 'h'), Some(43_200_000)),
            ("\"7d\"", after(7, 'd'), Some(604_800_000)),
            ("\"2w\"", after(2, 'w'), Some(1_209_600_000)),
            ("\"never\"", Some(DeleteArchivedAfter::Never), None),
            ("\"0\"", Some(DeleteArchivedAfter::Never), None),
            ("\"0d\"", Some(DeleteArchivedAfter::Never), None),
            ("0", Some(DeleteArchivedAfter::Never), None),
        ] {
            assert_eq!(archived(value), Ok(parsed), "{value}");
            assert_eq!(
                parsed.and_then(DeleteArchivedAfter::millis),
                millis,
                "{value}"
            );
        }
        assert_eq!(after(7, 'd').unwrap().to_string(), "7d", "as written");
        assert_eq!(DeleteArchivedAfter::Never.to_string(), "never");
        for wrong in [
            "\"7\"",
            "\"7x\"",
            "\"-1d\"",
            "\"+1d\"",
            "\"d\"",
            "\"\"",
            "\"7 d\"",
            "\"Never\"",
            "7",
            "true",
        ] {
            let error = archived(wrong).unwrap_err();
            assert!(error.contains("delete_archived_after"), "{wrong}: {error}");
        }
    }

    // spec: DMN-396, PRT-231
    #[test]
    fn delete_archived_after_is_written_back_into_the_daemon_table_and_served() {
        let set = |value: &str| SetSettingsParams {
            daemon: Some(DaemonSettings {
                delete_archived_after: Some(value.to_owned()),
            }),
            ..SetSettingsParams::default()
        };
        let text = write_back(FILE, &set("14d")).unwrap();
        assert!(
            text.contains("max_processes = 4 # enough\ndelete_archived_after = \"14d\"\n"),
            "{text}"
        );
        let mut expected = ConfigFile::parse(FILE).unwrap();
        expected.daemon.delete_archived_after = Some(DeleteArchivedAfter::After {
            count: 14,
            unit: 'd',
        });
        let file = ConfigFile::parse(&text).unwrap();
        assert_eq!(file, expected);
        assert_eq!(
            file.settings().daemon,
            Some(DaemonSettings {
                delete_archived_after: Some("14d".to_owned())
            })
        );
        assert_eq!(
            ConfigFile::parse(FILE).unwrap().settings().daemon,
            None,
            "absent: the default"
        );

        let text = write_back(&text, &set("0")).unwrap();
        assert!(
            text.contains("delete_archived_after = \"never\"\n"),
            "{text}"
        );
        let text = write_back("", &set("12h")).unwrap();
        assert_eq!(
            ConfigFile::parse(&text)
                .unwrap()
                .daemon
                .delete_archived_after,
            Some(DeleteArchivedAfter::After {
                count: 12,
                unit: 'h'
            })
        );
        let error = write_back(FILE, &set("7")).unwrap_err();
        assert!(error.contains("daemon.delete_archived_after"), "{error}");
    }
}
