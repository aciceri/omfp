//! Init files (DMN-185): `$XDG_CONFIG_HOME/omfp/init.ts` (else
//! `~/.config/omfp/init.ts`, the user's) and `<workspace>/.omfp/init.ts` (the workspace's) are ES modules every window
//! imports. The daemon versions them by content, publishes the versions on the `index` and `views/<id>` topics, and
//! serves them compiled like widget files at `<widgetBase>/init/<scope>/<version>/init.ts` (PRT-095).

use std::path::{Path, PathBuf};

use super::folders::Fnv;
use super::{OMFP_DIR, Widgets, compile};

/// The init file's name, in the config directory and in a workspace's `.omfp/`.
pub const FILE: &str = "init.ts";

/// `<workspace>/.omfp/init.ts`.
#[must_use]
pub fn workspace_init(workspace: &Path) -> PathBuf {
    workspace.join(OMFP_DIR).join(FILE)
}

/// The user's init file: `$XDG_CONFIG_HOME/omfp/init.ts` when absolute, else `$HOME/.config/omfp/init.ts`, next to
/// the theme file.
#[must_use]
pub fn default_init_file(
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    crate::config::config_dir(xdg_config_home, home).map(|dir| dir.join(FILE))
}

/// The version of an init file: FNV-1a 64 of its bytes as 16 lowercase hex digits.
#[must_use]
pub fn version(bytes: &[u8]) -> String {
    let mut hash = Fnv::default();
    hash.feed(bytes);
    hash.finish()
}

/// The version of the file at `path`; `None` when it is missing or cannot be read.
#[must_use]
pub fn read_version(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|bytes| version(&bytes))
}

impl Widgets {
    /// The init file at `path` compiled like a widget file (unknown `omfp/sdk` names are errors), or the stand-in
    /// module exporting `compileError` when it does not compile; `None` (404) when there is no such file.
    #[must_use]
    pub fn serve_init(&self, path: &Path) -> Option<String> {
        Some(
            self.compile_init(path)?
                .unwrap_or_else(|error| compile::stand_in(&error)),
        )
    }

    /// The compile result of the init file at `path` as [`Self::serve_init`] compiles it (DMN-197); `None` when there
    /// is no such file.
    #[must_use]
    pub fn check_init(&self, path: &Path) -> Option<Result<(), compile::CompileError>> {
        Some(self.compile_init(path)?.map(drop))
    }

    fn compile_init(&self, path: &Path) -> Option<Result<String, compile::CompileError>> {
        if !path.is_file() {
            return None;
        }
        Some(self.compile(path.parent()?, Path::new(path.file_name()?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: DMN-185
    #[test]
    fn the_init_files_live_in_the_config_directory_and_the_workspace() {
        assert_eq!(
            default_init_file(Some("/xdg".into()), Some("/home/u".into())),
            Some(PathBuf::from("/xdg/omfp/init.ts"))
        );
        assert_eq!(
            default_init_file(Some("relative".into()), Some("/home/u".into())),
            Some(PathBuf::from("/home/u/.config/omfp/init.ts"))
        );
        assert_eq!(default_init_file(None, None), None);
        assert_eq!(
            workspace_init(Path::new("/ws")),
            PathBuf::from("/ws/.omfp/init.ts")
        );
    }

    // spec: DMN-185
    #[test]
    fn the_version_is_the_fnv_hash_of_the_bytes() {
        // FNV-1a 64 test vectors.
        assert_eq!(version(b""), "cbf29ce484222325");
        assert_eq!(version(b"a"), "af63dc4c8601ec8c");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        assert_eq!(read_version(&path), None);
        std::fs::write(&path, "a").unwrap();
        assert_eq!(read_version(&path).as_deref(), Some("af63dc4c8601ec8c"));
    }

    // spec: DMN-185
    #[test]
    fn an_init_file_is_served_compiled_or_as_a_stand_in() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(
            bundle.join("widget-exports.json"),
            r#"{ "omfp/sdk": ["registerCommand"] }"#,
        )
        .unwrap();
        let widgets = Widgets::new(dir.path().join("library"), Some(&bundle));
        let path = dir.path().join(FILE);
        assert_eq!(widgets.serve_init(&path), None, "no file");

        std::fs::write(
            &path,
            "import { registerCommand } from \"omfp/sdk\";\nconst n: number = 1;\nregisterCommand(\"a\", () => n);\n",
        )
        .unwrap();
        let code = widgets.serve_init(&path).unwrap();
        assert!(code.contains("registerCommand"), "{code}");
        assert!(!code.contains(": number"), "types are stripped: {code}");
        assert!(!code.contains("compileError"), "{code}");

        std::fs::write(&path, "import { nothing } from \"omfp/sdk\";\n").unwrap();
        let stand_in = widgets.serve_init(&path).unwrap();
        assert!(stand_in.contains("compileError"), "{stand_in}");
        assert!(stand_in.contains("nothing"), "{stand_in}");

        assert_eq!(widgets.serve_init(dir.path()), None, "a folder");
    }

    // spec: DMN-197
    #[test]
    fn an_init_file_is_checked_as_it_is_served() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(
            bundle.join("widget-exports.json"),
            r#"{ "omfp/sdk": ["registerCommand"] }"#,
        )
        .unwrap();
        let widgets = Widgets::new(dir.path().join("library"), Some(&bundle));
        let path = dir.path().join(FILE);
        assert_eq!(widgets.check_init(&path), None, "no file");
        std::fs::write(
            &path,
            "import { registerCommand } from \"omfp/sdk\";\nregisterCommand(\"a\", () => 1);\n",
        )
        .unwrap();
        assert_eq!(widgets.check_init(&path), Some(Ok(())));
        std::fs::write(&path, "\nimport { nothing } from \"omfp/sdk\";\n").unwrap();
        let error = widgets.check_init(&path).unwrap().unwrap_err();
        assert_eq!(
            (error.file.as_str(), error.line, error.column),
            ("init.ts", Some(2), Some(10))
        );
        assert!(error.message.contains("nothing"), "{error:?}");
        assert_eq!(widgets.check_init(dir.path()), None, "a folder");
    }
}
