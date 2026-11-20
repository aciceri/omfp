//! Widget files to browser ES modules with oxc (DMN-084, DMN-085): types stripped, JSX on the automatic runtime
//! (`react/jsx-runtime`, resolved by the window's import map), an inline source map, no down-levelling. A file fails
//! on oxc's syntax and semantic errors, on an import that is neither relative (inside the folder) nor in the import
//! map, and on a name the bundle's module does not export; a failing widget is replaced by a stand-in module. A card's
//! one-file widget (PRT-225) compiles as `card.tsx` under the same rules, without relative imports.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_codegen::{Codegen, CodegenOptions};
use oxc_diagnostics::OxcDiagnostic;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_syntax::module_record::{ExportImportName, ImportImportName, ModuleRecord};
use oxc_transformer::{JsxOptions, JsxRuntime, TransformOptions, Transformer};
use serde::{Deserialize, Serialize};

/// The specifiers the window's import map resolves (AGT-012).
pub const IMPORT_MAP: [&str; 5] = [
    "react",
    "react/jsx-runtime",
    "react-dom",
    "omfp/sdk",
    "omfp/kit",
];

/// Export names of each import-map module, from the bundle's `widget-exports.json`.
pub type Exports = HashMap<String, HashSet<String>>;

/// Why a widget file does not compile (DMN-085): `file` relative to the widget folder, `line` and `column` 1-based.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompileError {
    pub file: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub message: String,
}

impl std::fmt::Display for CompileError {
    /// `file:line:column: message`, or `file: message` without a position.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.line, self.column) {
            (Some(line), Some(column)) => {
                write!(f, "{}:{line}:{column}: {}", self.file, self.message)
            }
            _ => write!(f, "{}: {}", self.file, self.message),
        }
    }
}

/// The file name a card's one-file widget compiles as (PRT-225).
pub const CARD_FILE: &str = "card.tsx";

/// Which imports besides the import map a file may have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Imports {
    /// Files of its widget folder (AGT-012).
    Folder,
    /// None: a card is one file (PRT-225).
    ImportMapOnly,
}

/// Parses `widget-exports.json`: `{ "<specifier>": ["name", …] }`.
pub fn parse_exports(text: &str) -> Result<Exports, serde_json::Error> {
    let lists: HashMap<String, Vec<String>> = serde_json::from_str(text)?;
    Ok(lists
        .into_iter()
        .map(|(specifier, names)| (specifier, names.into_iter().collect()))
        .collect())
}

/// Whether the daemon compiles this file (`.ts`, `.tsx`, `.mts`, `.jsx`) instead of serving it as it is.
#[must_use]
pub fn is_compiled(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("ts" | "tsx" | "mts" | "jsx")
    )
}

/// A relative path with `/` separators, as shown to the user.
#[must_use]
pub fn display(relative: &Path) -> String {
    relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Compiles `source`, the file `relative` of a widget folder, to an ES module with an inline source map; `exports`
/// lists what the import-map modules export (without it, imported names are not checked).
pub fn compile(
    relative: &Path,
    source: &str,
    exports: Option<&Exports>,
) -> Result<String, CompileError> {
    compile_with(relative, source, exports, Imports::Folder)
}

/// Compiles a card's one-file widget as [`CARD_FILE`] (PRT-225): [`compile`]'s rules, and no relative import.
pub fn compile_card(source: &str, exports: Option<&Exports>) -> Result<String, CompileError> {
    compile_with(
        Path::new(CARD_FILE),
        source,
        exports,
        Imports::ImportMapOnly,
    )
}

fn compile_with(
    relative: &Path,
    source: &str,
    exports: Option<&Exports>,
    imports: Imports,
) -> Result<String, CompileError> {
    let file = display(relative);
    let error_at = |offset: Option<u32>, message: String| {
        let (line, column) = offset.and_then(|offset| position(source, offset)).unzip();
        CompileError {
            file: file.clone(),
            line,
            column,
            message,
        }
    };
    let from_diagnostic = |diagnostic: &OxcDiagnostic| {
        let offset = diagnostic.labels.first().map(oxc_span::LabeledSpan::offset);
        error_at(offset, diagnostic.message.to_string())
    };
    let source_type =
        SourceType::from_path(relative).map_err(|error| error_at(None, error.to_string()))?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if let Some(diagnostic) = parsed.diagnostics.errors().next() {
        return Err(from_diagnostic(diagnostic));
    }
    let mut program = parsed.program;
    // Early errors the parser leaves to semantic analysis (duplicate `let`, a bad `break`, …).
    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .with_excess_capacity(2.0)
        .build(&program);
    if let Some(diagnostic) = semantic.diagnostics.errors().next() {
        return Err(from_diagnostic(diagnostic));
    }
    if let Some((offset, message)) =
        import_errors(relative, source, &parsed.module_record, exports, imports)
            .into_iter()
            .min_by_key(|(offset, _)| *offset)
    {
        return Err(error_at(Some(offset), message));
    }
    let options = TransformOptions {
        jsx: JsxOptions {
            runtime: JsxRuntime::Automatic,
            development: false,
            ..JsxOptions::enable()
        },
        ..TransformOptions::default()
    };
    let transformed = Transformer::new(&allocator, relative, &options)
        .build_with_scoping(semantic.semantic.into_scoping(), &mut program);
    if let Some(diagnostic) = transformed.diagnostics.errors().next() {
        return Err(from_diagnostic(diagnostic));
    }
    let out = Codegen::new()
        .with_options(CodegenOptions {
            source_map_path: Some(relative.to_path_buf()),
            ..CodegenOptions::default()
        })
        .build(&program);
    Ok(match out.map {
        Some(map) => format!("{}//# sourceMappingURL={}\n", out.code, map.to_data_url()),
        None => out.code,
    })
}

/// Every import of the module that breaks AGT-012, with its byte offset: a specifier neither relative (inside the
/// folder, when `imports` allows files of the folder) nor in the import map, or a name its import-map module does not
/// export.
fn import_errors(
    relative: &Path,
    source: &str,
    record: &ModuleRecord<'_>,
    exports: Option<&Exports>,
    imports: Imports,
) -> Vec<(u32, String)> {
    let mut errors = Vec::new();
    let mut check_specifier = |specifier: &str, offset: u32| {
        if let Some(reason) = specifier_error(relative, specifier, imports) {
            errors.push((offset, reason));
        }
    };
    for (specifier, requests) in &record.requested_modules {
        for request in requests.iter().filter(|request| !request.is_type) {
            check_specifier(specifier, request.span.start);
        }
    }
    for import in &record.dynamic_imports {
        let text = source
            .get(import.module_request.start as usize..import.module_request.end as usize)
            .unwrap_or_default();
        if let Some(specifier) = string_literal(text) {
            check_specifier(specifier, import.module_request.start);
        }
    }
    let Some(exports) = exports else {
        return errors;
    };
    let mut check_name = |specifier: &str, name: Option<&str>, offset: u32| {
        let Some(names) = exports.get(specifier) else {
            return;
        };
        let name = name.unwrap_or("default");
        if !names.contains(name) {
            let reason = if name == "default" {
                format!("{specifier} has no default export")
            } else {
                format!("{specifier} has no export named {name}")
            };
            errors.push((offset, reason));
        }
    };
    for import in record
        .import_entries
        .iter()
        .filter(|import| !import.is_type)
    {
        let specifier = &import.module_request.name;
        match &import.import_name {
            ImportImportName::Name(name) => {
                check_name(specifier, Some(&name.name), name.span.start);
            }
            ImportImportName::Default(span) => check_name(specifier, None, span.start),
            ImportImportName::NamespaceObject => {}
        }
    }
    for export in record
        .indirect_export_entries
        .iter()
        .filter(|export| !export.is_type)
    {
        if let (Some(request), ExportImportName::Name(name)) =
            (&export.module_request, &export.import_name)
        {
            let imported = (name.name.as_str() != "default").then_some(name.name.as_str());
            check_name(&request.name, imported, name.span.start);
        }
    }
    errors
}

/// Why a widget file may not import `specifier`, if it may not.
fn specifier_error(relative: &Path, specifier: &str, imports: Imports) -> Option<String> {
    if IMPORT_MAP.contains(&specifier) {
        return None;
    }
    if imports == Imports::ImportMapOnly {
        return Some(format!(
            "cannot import \"{specifier}\": a card is one file and imports only {}",
            IMPORT_MAP.join(", ")
        ));
    }
    if specifier.starts_with("./") || specifier.starts_with("../") {
        return leaves_folder(relative, specifier)
            .then(|| format!("cannot import \"{specifier}\": it leaves the widget folder"));
    }
    Some(format!(
        "cannot import \"{specifier}\": a widget imports files of its own folder (./…) and {}",
        IMPORT_MAP.join(", ")
    ))
}

/// Whether the relative `specifier` of the folder's file `relative` points outside the folder.
fn leaves_folder(relative: &Path, specifier: &str) -> bool {
    let mut at: PathBuf = relative.parent().map(Path::to_path_buf).unwrap_or_default();
    for component in Path::new(specifier).components() {
        match component {
            Component::ParentDir if !at.pop() => return true,
            Component::Normal(part) => at.push(part),
            _ => {}
        }
    }
    false
}

/// The value of a JavaScript string literal without escapes or substitutions (`"x"`, `'x'`, `` `x` ``).
fn string_literal(text: &str) -> Option<&str> {
    let quote = text.chars().next()?;
    let inner = text
        .strip_prefix(quote)?
        .strip_suffix(quote)
        .filter(|_| matches!(quote, '"' | '\'' | '`'))?;
    (!inner.contains(['\\', quote]) && !inner.contains("${")).then_some(inner)
}

/// 1-based line and column (in characters) of byte `offset` of `source`.
fn position(source: &str, offset: u32) -> Option<(u32, u32)> {
    let before = source.get(..offset as usize)?;
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit_once('\n')
        .map_or(before, |(_, last)| last)
        .chars()
        .count()
        + 1;
    Some((u32::try_from(line).ok()?, u32::try_from(column).ok()?))
}

/// The module served in place of a widget that does not compile (DMN-085): `compileError` plus a default component
/// rendering it.
#[must_use]
pub fn stand_in(error: &CompileError) -> String {
    let error_json = serde_json::to_string(error).unwrap_or_else(|_| "null".to_owned());
    let text = serde_json::Value::String(error.to_string());
    format!(
        "import {{ createElement }} from \"react\";\n\
         export const compileError = {error_json};\n\
         export default function CompileError() {{\n  \
         return createElement(\"pre\", {{ role: \"alert\", \"data-compile-error\": \"\" }}, {text});\n\
         }}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exports() -> Exports {
        parse_exports(
            r#"{ "react": ["useState", "createElement", "default"], "react/jsx-runtime": ["jsx", "jsxs", "Fragment"],
                 "omfp/kit": ["Card", "Progress", "Log"], "omfp/sdk": ["useJob"] }"#,
        )
        .unwrap()
    }

    fn compiled(file: &str, source: &str) -> Result<String, CompileError> {
        compile(Path::new(file), source, Some(&exports()))
    }

    // spec: DMN-084
    #[test]
    fn typescript_and_jsx_become_modules_on_the_automatic_runtime_with_an_inline_source_map() {
        let code = compiled(
            "index.tsx",
            "import { Card } from \"omfp/kit\";\nimport { helper } from \"./lib/helper\";\n\
             type Props = { title: string };\n\
             export default function W({ title }: Props) { return <Card title={title}>{helper()}</Card>; }\n",
        )
        .unwrap();
        assert!(code.contains("from \"react/jsx-runtime\""), "{code}");
        assert!(code.contains("from \"./lib/helper\""), "{code}");
        assert!(!code.contains("Props"), "types stripped: {code}");
        assert!(
            code.contains("//# sourceMappingURL=data:application/json;"),
            "{code}"
        );
    }

    // spec: AGT-014, DMN-085
    #[test]
    fn a_syntax_error_has_its_file_line_and_column() {
        let error = compiled("lib/broken.ts", "export const a = 1;\nconst b = ;\n").unwrap_err();
        assert_eq!(
            (error.file.as_str(), error.line, error.column),
            ("lib/broken.ts", Some(2), Some(11))
        );
        assert_ne!(error.message, "");

        let semantic = compiled("a.ts", "let x = 1;\nlet x = 2;\n").unwrap_err();
        // oxc's first label: the first declaration.
        assert_eq!(
            (semantic.line, semantic.column),
            (Some(1), Some(5)),
            "{semantic:?}"
        );
        assert_eq!(semantic.message, "Identifier `x` has already been declared");
    }

    // spec: AGT-012, AGT-014, DMN-085
    #[test]
    fn only_relative_files_inside_the_folder_and_the_import_map_may_be_imported() {
        for allowed in [
            "import { useState } from \"react\"; import \"./x.css\"; export {};",
            "import { createRoot } from \"react-dom\"; import x from \"../up\"; export {};",
            "import type { Lodash } from \"lodash\"; export const a: Lodash | null = null;",
        ] {
            let file = if allowed.contains("../up") {
                "lib/a.ts"
            } else {
                "a.ts"
            };
            assert!(compiled(file, allowed).is_ok(), "{allowed}");
        }

        let error = compiled("a.ts", "export {};\nimport _ from \"lodash\";\n").unwrap_err();
        assert_eq!((error.line, error.column), (Some(2), Some(15)));
        assert!(
            error.message.starts_with("cannot import \"lodash\""),
            "{error:?}"
        );
        let error = compiled("a.ts", "import x from \"../outside\"; export { x };").unwrap_err();
        assert!(
            error.message.contains("leaves the widget folder"),
            "{error:?}"
        );
        let error = compiled("a.ts", "export const m = import(\"https://cdn/x.js\");").unwrap_err();
        assert!(error.message.contains("https://cdn/x.js"), "{error:?}");
    }

    // spec: AGT-014, DMN-085
    #[test]
    fn names_the_import_map_modules_do_not_export_are_errors() {
        let error = compiled(
            "a.tsx",
            "import { Card, Gauge } from \"omfp/kit\";\nexport default () => <Card><Gauge /></Card>;\n",
        )
        .unwrap_err();
        assert_eq!(error.message, "omfp/kit has no export named Gauge");
        assert_eq!((error.line, error.column), (Some(1), Some(16)));

        let error = compiled("a.ts", "import kit from \"omfp/kit\"; export { kit };").unwrap_err();
        assert_eq!(error.message, "omfp/kit has no default export");
        let error = compiled("a.ts", "export { useNothing } from \"omfp/sdk\";").unwrap_err();
        assert_eq!(error.message, "omfp/sdk has no export named useNothing");

        // Namespaces, default exports that exist, and modules the file does not list pass.
        for fine in [
            "import * as kit from \"omfp/kit\"; export { kit };",
            "import React from \"react\"; export { React };",
            "import { anything } from \"react-dom\"; export { anything };",
        ] {
            assert!(compiled("a.ts", fine).is_ok(), "{fine}");
        }
        // Without the bundle's export list only specifiers are checked.
        assert!(
            compile(
                Path::new("a.ts"),
                "import { Gauge } from \"omfp/kit\"; export { Gauge };",
                None
            )
            .is_ok()
        );
    }

    // spec: DMN-085
    #[test]
    fn the_stand_in_exports_the_error_and_renders_it_with_react() {
        let error = CompileError {
            file: "lib/chart.ts".into(),
            line: Some(3),
            column: Some(7),
            message: "Unexpected \"token\"".into(),
        };
        let module = stand_in(&error);
        assert!(module.starts_with("import { createElement } from \"react\";\n"));
        assert!(module.contains(
            "export const compileError = {\"file\":\"lib/chart.ts\",\"line\":3,\"column\":7,\"message\":\"Unexpected \\\"token\\\"\"};"
        ), "{module}");
        assert!(module.contains("export default function CompileError()"));
        assert!(
            module.contains("\"lib/chart.ts:3:7: Unexpected \\\"token\\\"\""),
            "{module}"
        );
        // The stand-in itself compiles and imports only from the import map.
        assert!(compiled("stand-in.js", &module).is_ok());
    }

    // spec: PRT-225, AGT-096
    #[test]
    fn a_card_compiles_as_card_tsx_and_imports_only_from_the_import_map() {
        let module = compile_card(
            "import { Card } from \"omfp/kit\";\nimport { useAnswer } from \"omfp/sdk\";\n\
             export default function Ask({ q }: { q: string }) { const a = useAnswer(); return <Card title={q}>{a.state}</Card>; }\n",
            None,
        )
        .unwrap();
        assert!(module.contains("from \"react/jsx-runtime\""), "{module}");
        assert!(
            module.contains("//# sourceMappingURL=data:application/json;"),
            "{module}"
        );

        let error = compile_card(
            "export {};\nimport { helper } from \"./helper\";\n",
            Some(&exports()),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "card.tsx:2:24: cannot import \"./helper\": a card is one file and imports only react, \
             react/jsx-runtime, react-dom, omfp/sdk, omfp/kit"
        );
        let error = compile_card("const = ;", None).unwrap_err();
        assert_eq!((error.file.as_str(), error.line), (CARD_FILE, Some(1)));
        assert!(error.to_string().starts_with("card.tsx:1:"), "{error}");
        let error = compile_card(
            "import { Gauge } from \"omfp/kit\"; export { Gauge };",
            Some(&exports()),
        )
        .unwrap_err();
        assert_eq!(error.message, "omfp/kit has no export named Gauge");
    }
}
