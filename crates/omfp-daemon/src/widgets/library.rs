//! The component library (AGT-064, AGT-065): the user library `<data-dir>/widgets/` (AGT-023), saved into from a
//! workspace widget or inline code, and listed with each component's description.

use std::path::Path;

use omfp_protocol::{LibraryComponent, WidgetSource};

use super::folders::{self, checked_name};
use super::{CopyError, compile, copy_dir};
use crate::store::new_id;

/// Longest description `widgets.library` gives, in characters (AGT-065).
pub const MAX_DESCRIPTION_CHARS: usize = 200;

/// What a component is saved from (AGT-064).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source<'a> {
    /// A widget folder, copied whole.
    Folder(&'a Path),
    /// A component's source, saved as `index.tsx`.
    Code(&'a str),
}

/// Saves `source` as the component `name` of `library`: staged under a hidden name, then renamed, so the folder
/// appears whole; an existing component is replaced only with `replace` (AGT-064).
pub fn save(
    library: &Path,
    name: &str,
    source: Source<'_>,
    replace: bool,
) -> Result<(), CopyError> {
    checked_name("component name", name).map_err(CopyError::BadRequest)?;
    if let Source::Folder(folder) = source
        && !folder.is_dir()
    {
        return Err(CopyError::NotFound(format!(
            "no widget folder {}",
            folder.display()
        )));
    }
    let target = library.join(name);
    let exists = || std::fs::symlink_metadata(&target).is_ok();
    let conflict = || {
        CopyError::Conflict(format!(
            "the library already has a component {name}; save with replace: true to replace it"
        ))
    };
    if exists() && !replace {
        return Err(conflict());
    }
    let io = |error: std::io::Error| CopyError::Io(format!("cannot save {name}: {error}"));
    let staging = library.join(format!(".{name}.{}.tmp", new_id()));
    let staged = std::fs::create_dir_all(library).and_then(|()| match source {
        Source::Folder(folder) => copy_dir(folder, &staging),
        Source::Code(code) => std::fs::create_dir(&staging)
            .and_then(|()| std::fs::write(staging.join("index.tsx"), code)),
    });
    if let Err(error) = staged {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(io(error));
    }
    // The old folder moves aside first; it is deleted once the new one is in place.
    let old = library.join(format!(".{name}.{}.old", new_id()));
    let moved_aside = replace && exists();
    if moved_aside && let Err(error) = std::fs::rename(&target, &old) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(io(error));
    }
    if let Err(error) = std::fs::rename(&staging, &target) {
        let _ = std::fs::remove_dir_all(&staging);
        if moved_aside {
            let _ = std::fs::rename(&old, &target);
        }
        return Err(if exists() { conflict() } else { io(error) });
    }
    if moved_aside {
        let _ = std::fs::remove_dir_all(&old);
    }
    Ok(())
}

/// The components of `library` by name, with their files and descriptions (AGT-065).
#[must_use]
pub fn list(library: &Path) -> Vec<LibraryComponent> {
    folders::scan(library, WidgetSource::User)
        .into_iter()
        .map(|widget| {
            let folder = library.join(&widget.name);
            let description = widget
                .entry
                .as_ref()
                .and_then(|entry| std::fs::read_to_string(folder.join(entry)).ok())
                .map(|source| description(&source))
                .unwrap_or_default();
            LibraryComponent {
                reference: format!("user:{}", widget.name),
                files: folders::files(&folder)
                    .iter()
                    .map(|(path, _)| compile::display(path))
                    .collect(),
                name: widget.name,
                entry: widget.entry,
                version: widget.version,
                description,
            }
        })
        .collect()
}

/// The leading comment of `source` (its `//` lines or first `/* … */` block), whitespace collapsed, at most
/// [`MAX_DESCRIPTION_CHARS`] characters (cut with `…`); empty without one (AGT-065).
#[must_use]
pub fn description(source: &str) -> String {
    let source = source.trim_start_matches('\u{feff}').trim_start();
    let text = if let Some(block) = source.strip_prefix("/*") {
        let body = block.split_once("*/").map_or(block, |(body, _)| body);
        body.lines()
            .map(|line| line.trim().trim_start_matches('*'))
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        source
            .lines()
            .map(str::trim)
            .take_while(|line| line.starts_with("//"))
            .map(|line| line.trim_start_matches('/'))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > MAX_DESCRIPTION_CHARS {
        let mut cut: String = collapsed.chars().take(MAX_DESCRIPTION_CHARS - 1).collect();
        cut.push('…');
        cut
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: AGT-065
    #[test]
    fn a_description_is_the_leading_comment_collapsed_and_cut() {
        assert_eq!(
            description(
                "// Threads of a feature.\n//   props: threads\nimport x from \"react\";\n"
            ),
            "Threads of a feature. props: threads"
        );
        assert_eq!(
            description("\n/**\n * A clock.\n * props: zone\n */\nexport default 1;\n// later\n"),
            "A clock. props: zone"
        );
        assert_eq!(description("export default 1; // not leading\n"), "");
        let long = format!("// {}", "word ".repeat(100));
        let cut = description(&long);
        assert_eq!(cut.chars().count(), MAX_DESCRIPTION_CHARS);
        assert!(cut.ends_with('…'), "{cut}");
    }

    // spec: AGT-064, AGT-065
    #[test]
    fn saving_copies_a_folder_or_writes_code_and_replaces_only_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("widgets");
        let widget = dir.path().join("workspace/.omfp/widgets/board");
        std::fs::create_dir_all(widget.join("parts")).unwrap();
        std::fs::write(widget.join("index.tsx"), "// A board.\nexport default 1;\n").unwrap();
        std::fs::write(widget.join("parts/row.tsx"), "export default 2;\n").unwrap();

        save(&library, "board", Source::Folder(&widget), false).unwrap();
        save(
            &library,
            "clock",
            Source::Code("/* A clock. */ export default 3;\n"),
            false,
        )
        .unwrap();
        let listed = list(&library);
        assert_eq!(
            listed
                .iter()
                .map(|component| (
                    component.reference.as_str(),
                    component.entry.as_deref(),
                    component.files.clone(),
                    component.description.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    "user:board",
                    Some("index.tsx"),
                    vec!["index.tsx".to_owned(), "parts/row.tsx".to_owned()],
                    "A board."
                ),
                (
                    "user:clock",
                    Some("index.tsx"),
                    vec!["index.tsx".to_owned()],
                    "A clock."
                ),
            ]
        );

        let error = save(
            &library,
            "clock",
            Source::Code("export default 4;\n"),
            false,
        )
        .unwrap_err();
        assert!(
            matches!(&error, CopyError::Conflict(text) if text.contains("replace: true")),
            "{error}"
        );
        save(
            &library,
            "clock",
            Source::Code("// New clock.\nexport default 4;\n"),
            true,
        )
        .unwrap();
        assert_eq!(list(&library)[1].description, "New clock.");
        let hidden: Vec<String> = std::fs::read_dir(&library)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with('.'))
            .collect();
        assert_eq!(hidden, Vec::<String>::new(), "no staging left");

        assert!(matches!(
            save(&library, ".hidden", Source::Code(""), false),
            Err(CopyError::BadRequest(_))
        ));
        assert!(matches!(
            save(
                &library,
                "gone",
                Source::Folder(&dir.path().join("nope")),
                false
            ),
            Err(CopyError::NotFound(_))
        ));
    }
}
