//! Resolve proto `import` statements by inlining imported definitions.
//!
//! The JobWorkerP plugin system requires each proto string to be
//! self-contained (no `import`), with the first `message` being the primary
//! type. This module provides [`resolve_proto_imports`] which takes a main
//! proto and its imports (path→content pairs), strips import lines, and
//! appends the imported definitions after the main content so the primary
//! message stays first.
//!
//! Package-prefix removal targets only proto3 type-reference positions
//! (field types, map value types, rpc argument/return types, extend targets)
//! using regex-based matching. Quoted strings, comments, package/syntax
//! declarations, and non-type-reference positions are never modified.

use regex::Regex;

/// Resolve proto `import` statements by inlining the imported definitions,
/// producing a self-contained proto string.
///
/// # Errors
///
/// Returns `Err` if the resolved output still contains unresolved `import`
/// statements, either from the main proto or from any of the imported files
/// (i.e. transitive imports that were not provided in `imports`).
pub fn resolve_proto_imports(main_proto: &str, imports: &[(&str, &str)]) -> Result<String, String> {
    let import_paths: Vec<&str> = imports.iter().map(|(p, _)| *p).collect();

    let imported_packages: Vec<String> = imports
        .iter()
        .filter_map(|(_, content)| extract_package(content))
        .collect();

    // Strip resolved import lines from main proto
    let mut main_lines: Vec<&str> = Vec::new();
    for line in main_proto.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("import ")
            && extract_import_path(trimmed)
                .is_some_and(|path| import_paths.contains(&path.as_str()))
        {
            continue;
        }
        main_lines.push(line);
    }

    let mut output = main_lines.join("\n");

    // Append each imported file's definitions.
    // Strip syntax/package lines; strip only resolved import lines (keep
    // unresolved ones so the final check catches transitive imports).
    for (_path, content) in imports {
        output.push('\n');
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("syntax ") || trimmed.starts_with("package ") {
                continue;
            }
            if trimmed.starts_with("import ")
                && extract_import_path(trimmed)
                    .is_some_and(|path| import_paths.contains(&path.as_str()))
            {
                continue;
            }
            output.push_str(line);
            output.push('\n');
        }
    }

    // Replace package-qualified type references with local names
    for pkg in &imported_packages {
        output = strip_package_prefix_in_type_refs(&output, pkg);
    }

    // Fail-fast: reject if any import statements remain unresolved
    let unresolved: Vec<&str> = output
        .lines()
        .filter(|l| l.trim().starts_with("import "))
        .collect();
    if !unresolved.is_empty() {
        return Err(format!(
            "unresolved import(s) remain after resolution — \
             all transitive imports must be provided in the `imports` table:\n{}",
            unresolved.join("\n")
        ));
    }

    Ok(output)
}

/// Extract the quoted path from an `import "...";` line.
fn extract_import_path(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let rest = trimmed
        .strip_prefix("import public ")
        .or_else(|| trimmed.strip_prefix("import "))?;
    let start = rest.find('"')?;
    let end = rest[start + 1..].find('"')?;
    Some(rest[start + 1..start + 1 + end].to_string())
}

/// Extract the `package` name from a proto source string.
fn extract_package(proto: &str) -> Option<String> {
    proto.lines().find_map(|line| {
        let trimmed = line.trim();
        trimmed
            .strip_prefix("package ")
            .and_then(|rest| rest.strip_suffix(';'))
            .map(|pkg| pkg.trim().to_string())
    })
}

/// Remove a package prefix from type references in proto3 syntactic positions.
///
/// Targets these proto3 type-reference contexts:
/// 1. Field types: `(optional|repeated)? pkg.Type field_name = N;`
/// 2. Map value types: `map<KeyType, pkg.Type>`
/// 3. RPC arguments/returns: `rpc Name(pkg.Type) returns (pkg.Type)`
/// 4. Extend targets: `extend pkg.Type {`
///
/// Processing is line-by-line: comment-only lines and trailing comments are
/// preserved verbatim; regex replacement is applied only to the code portion.
/// Nested type names (e.g. `pkg.Outer.Inner`) are supported.
fn strip_package_prefix_in_type_refs(proto: &str, package: &str) -> String {
    let escaped = regex::escape(package);
    let prefix_pattern = format!(r"{escaped}\.");

    let field_re = field_type_regex(&prefix_pattern);
    let map_re = map_value_regex(&prefix_pattern);
    let rpc_re = rpc_type_regex(&prefix_pattern);
    let extend_re = extend_regex(&prefix_pattern);
    let all_regexes: [&Regex; 4] = [&field_re, &map_re, &rpc_re, &extend_re];

    proto
        .lines()
        .map(|line| {
            let trimmed = line.trim();

            // Skip full-line comments entirely
            if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
                return line.to_string();
            }

            // Split code from trailing comment
            let (code, comment) = split_trailing_comment(line);

            // Apply all regex replacements to code portion only
            let mut replaced = code.to_string();
            for re in all_regexes {
                replaced = re.replace_all(&replaced, "$pre$type$post").to_string();
            }

            match comment {
                Some(c) => format!("{replaced}{c}"),
                None => replaced,
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Split a line into (code, trailing_comment) at the first `//` that is
/// not inside a quoted string. Handles backslash-escaped quotes (`\"`).
fn split_trailing_comment(line: &str) -> (&str, Option<&str>) {
    let mut in_quotes = false;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if in_quotes && bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2; // skip escaped character
            continue;
        }
        if bytes[i] == b'"' {
            in_quotes = !in_quotes;
        } else if !in_quotes && bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            return (&line[..i], Some(&line[i..]));
        }
        i += 1;
    }
    (line, None)
}

/// Type name pattern that supports nested types (e.g. `Outer.Inner`).
const TYPE_NAME_PATTERN: &str = r"[A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*";

fn field_type_regex(prefix_pattern: &str) -> Regex {
    let pattern = format!(
        r"(?P<pre>(?:optional\s+|repeated\s+|required\s+)?){prefix_pattern}(?P<type>{TYPE_NAME_PATTERN})(?P<post>\s+\w+\s*=)"
    );
    Regex::new(&pattern).expect("invalid field regex")
}

fn map_value_regex(prefix_pattern: &str) -> Regex {
    let pattern = format!(
        r"(?P<pre>map\s*<\s*\w+\s*,\s*){prefix_pattern}(?P<type>{TYPE_NAME_PATTERN})(?P<post>\s*>)"
    );
    Regex::new(&pattern).expect("invalid map regex")
}

fn rpc_type_regex(prefix_pattern: &str) -> Regex {
    let pattern =
        format!(r"(?P<pre>\(\s*){prefix_pattern}(?P<type>{TYPE_NAME_PATTERN})(?P<post>\s*\))");
    Regex::new(&pattern).expect("invalid rpc regex")
}

fn extend_regex(prefix_pattern: &str) -> Regex {
    let pattern =
        format!(r"(?P<pre>extend\s+){prefix_pattern}(?P<type>{TYPE_NAME_PATTERN})(?P<post>\s*\{{)");
    Regex::new(&pattern).expect("invalid extend regex")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_import_statement(proto: &str) -> bool {
        !proto.lines().any(|l| l.trim().starts_with("import "))
    }

    const MAIN_WITH_IMPORT: &str = "\
syntax = \"proto3\";

package test_pkg;

import \"llama_cpp/media_input.proto\";

message MainMessage {
  string name = 1;
  llama_cpp.MediaInput media = 2;
}";

    const IMPORTED: &str = "\
syntax = \"proto3\";

package llama_cpp;

message MediaInput {
  bytes data = 1;
}

enum MediaKind {
  MEDIA_KIND_UNSPECIFIED = 0;
}";

    fn resolve(main: &str, imports: &[(&str, &str)]) -> String {
        resolve_proto_imports(main, imports).expect("resolve failed")
    }

    #[test]
    fn import_lines_are_removed() {
        let result = resolve(
            MAIN_WITH_IMPORT,
            &[("llama_cpp/media_input.proto", IMPORTED)],
        );
        assert!(
            !result.contains("import "),
            "resolved proto must not contain import statements"
        );
    }

    #[test]
    fn first_message_is_primary() {
        let result = resolve(
            MAIN_WITH_IMPORT,
            &[("llama_cpp/media_input.proto", IMPORTED)],
        );
        let first_msg = result
            .lines()
            .find(|l| l.trim().starts_with("message "))
            .unwrap();
        assert!(
            first_msg.contains("MainMessage"),
            "first message must be the primary type from main proto, got: {first_msg}"
        );
    }

    #[test]
    fn cross_package_prefix_removed_from_type_refs() {
        let result = resolve(
            MAIN_WITH_IMPORT,
            &[("llama_cpp/media_input.proto", IMPORTED)],
        );
        assert!(
            !result
                .lines()
                .any(|l| l.trim().starts_with("llama_cpp.") || l.contains(" llama_cpp.Media")),
            "cross-package prefix should be stripped from type references"
        );
        assert!(
            result.contains("MediaInput media"),
            "type name without prefix should remain"
        );
    }

    #[test]
    fn package_declaration_preserved() {
        let result = resolve(
            MAIN_WITH_IMPORT,
            &[("llama_cpp/media_input.proto", IMPORTED)],
        );
        assert!(
            result.contains("package test_pkg;"),
            "main package declaration must be preserved"
        );
    }

    #[test]
    fn imported_syntax_and_package_stripped() {
        let result = resolve(
            MAIN_WITH_IMPORT,
            &[("llama_cpp/media_input.proto", IMPORTED)],
        );
        let syntax_count = result.matches("syntax ").count();
        assert_eq!(syntax_count, 1, "only one syntax declaration expected");
        assert!(
            !result.contains("package llama_cpp;"),
            "imported package declaration should be stripped"
        );
    }

    #[test]
    fn imported_definitions_are_present() {
        let result = resolve(
            MAIN_WITH_IMPORT,
            &[("llama_cpp/media_input.proto", IMPORTED)],
        );
        assert!(
            result.contains("message MediaInput"),
            "imported message must be inlined"
        );
        assert!(
            result.contains("enum MediaKind"),
            "imported enum must be inlined"
        );
    }

    #[test]
    fn no_import_returns_unchanged() {
        let no_import = "syntax = \"proto3\";\n\nmessage Simple { string x = 1; }";
        let result = resolve(no_import, &[]);
        assert_eq!(result, no_import);
    }

    #[test]
    fn error_on_unresolved_import() {
        let proto = "\
syntax = \"proto3\";\nimport \"missing/dep.proto\";\nmessage Foo { string x = 1; }";
        let err = resolve_proto_imports(proto, &[]).unwrap_err();
        assert!(err.contains("unresolved import"), "got: {err}");
    }

    #[test]
    fn error_on_transitive_unresolved_import() {
        let main = "\
syntax = \"proto3\";\nimport \"a.proto\";\nmessage Main { A a = 1; }";
        let a_proto = "\
syntax = \"proto3\";\npackage a;\nimport \"b.proto\";\nmessage A { string x = 1; }";
        let err = resolve_proto_imports(main, &[("a.proto", a_proto)]).unwrap_err();
        assert!(err.contains("unresolved import"), "got: {err}");
    }

    #[test]
    fn exact_path_match_no_partial() {
        let proto = "\
syntax = \"proto3\";\n\
import \"llama_cpp/media_input.proto.v2\";\n\
message Msg { string x = 1; }";
        let err =
            resolve_proto_imports(proto, &[("llama_cpp/media_input.proto", IMPORTED)]).unwrap_err();
        assert!(
            err.contains("unresolved import"),
            "partial path match should not resolve: {err}"
        );
    }

    #[test]
    fn exact_path_match_no_prefix() {
        let proto = "\
syntax = \"proto3\";\n\
import \"vendor/llama_cpp/media_input.proto\";\n\
message Msg { string x = 1; }";
        let err =
            resolve_proto_imports(proto, &[("llama_cpp/media_input.proto", IMPORTED)]).unwrap_err();
        assert!(
            err.contains("unresolved import"),
            "prefixed path should not resolve: {err}"
        );
    }

    #[test]
    fn prefix_not_removed_from_comments() {
        let proto = "syntax = \"proto3\";\n\
                      package test;\n\
                      import \"llama_cpp/media_input.proto\";\n\
                      // This field uses llama_cpp.MediaInput type.\n\
                      message Msg {\n\
                        llama_cpp.MediaInput m = 1;\n\
                      }";
        let result = resolve(proto, &[("llama_cpp/media_input.proto", IMPORTED)]);
        assert!(
            result.contains("// This field uses llama_cpp.MediaInput type"),
            "prefix in comments should not be stripped, got:\n{result}"
        );
        assert!(
            result.contains("MediaInput m = 1;"),
            "prefix in type reference should be stripped, got:\n{result}"
        );
    }

    #[test]
    fn prefix_not_removed_from_trailing_comment() {
        let proto = "syntax = \"proto3\";\n\
                      package test;\n\
                      import \"llama_cpp/media_input.proto\";\n\
                      message Msg {\n\
                        llama_cpp.MediaInput m = 1; // ref llama_cpp.MediaInput\n\
                      }";
        let result = resolve(proto, &[("llama_cpp/media_input.proto", IMPORTED)]);
        assert!(
            result.contains("// ref llama_cpp.MediaInput"),
            "prefix in trailing comment should not be stripped, got:\n{result}"
        );
        assert!(
            result.contains("MediaInput m = 1;"),
            "prefix in type reference should be stripped, got:\n{result}"
        );
    }

    #[test]
    fn prefix_not_removed_from_quoted_string() {
        let proto = "syntax = \"proto3\";\n\
                      package test;\n\
                      import \"llama_cpp/media_input.proto\";\n\
                      message Msg {\n\
                        llama_cpp.MediaInput m = 1;\n\
                        string label = 2 [default = \"llama_cpp.MediaInput\"];\n\
                      }";
        let result = resolve(proto, &[("llama_cpp/media_input.proto", IMPORTED)]);
        assert!(
            result.contains("\"llama_cpp.MediaInput\""),
            "prefix in quoted string should not be stripped, got:\n{result}"
        );
        assert!(
            result.contains("MediaInput m = 1;"),
            "prefix in type ref should be stripped, got:\n{result}"
        );
    }

    #[test]
    fn prefix_removed_in_rpc_types() {
        let proto = "syntax = \"proto3\";\n\
                      package svc;\n\
                      import \"llama_cpp/media_input.proto\";\n\
                      service MediaService {\n\
                        rpc Process(llama_cpp.MediaInput) returns (llama_cpp.MediaInput);\n\
                      }";
        let result = resolve(proto, &[("llama_cpp/media_input.proto", IMPORTED)]);
        assert!(
            result.contains("rpc Process(MediaInput) returns (MediaInput)"),
            "prefix in rpc types should be stripped, got:\n{result}"
        );
    }

    #[test]
    fn prefix_removed_in_map_value_type() {
        let proto = "syntax = \"proto3\";\n\
                      package test;\n\
                      import \"llama_cpp/media_input.proto\";\n\
                      message Msg {\n\
                        map<string, llama_cpp.MediaInput> items = 1;\n\
                      }";
        let result = resolve(proto, &[("llama_cpp/media_input.proto", IMPORTED)]);
        assert!(
            result.contains("map<string, MediaInput>"),
            "prefix in map value type should be stripped, got:\n{result}"
        );
    }

    #[test]
    fn prefix_removed_in_extend() {
        let proto = "syntax = \"proto3\";\n\
                      package test;\n\
                      import \"llama_cpp/media_input.proto\";\n\
                      extend llama_cpp.MediaInput {\n\
                        optional string ext_field = 100;\n\
                      }";
        let result = resolve(proto, &[("llama_cpp/media_input.proto", IMPORTED)]);
        assert!(
            result.contains("extend MediaInput {"),
            "prefix in extend should be stripped, got:\n{result}"
        );
    }

    #[test]
    fn prefix_removed_with_optional_qualifier() {
        let proto = "syntax = \"proto3\";\n\
                      package test;\n\
                      import \"llama_cpp/media_input.proto\";\n\
                      message Msg {\n\
                        optional llama_cpp.MediaInput m = 1;\n\
                        repeated llama_cpp.MediaInput ms = 2;\n\
                      }";
        let result = resolve(proto, &[("llama_cpp/media_input.proto", IMPORTED)]);
        assert!(
            result.contains("optional MediaInput m = 1;"),
            "prefix after optional should be stripped, got:\n{result}"
        );
        assert!(
            result.contains("repeated MediaInput ms = 2;"),
            "prefix after repeated should be stripped, got:\n{result}"
        );
    }

    #[test]
    fn nested_type_name_resolved() {
        let imported_with_nested = "\
syntax = \"proto3\";\n\
package outer_pkg;\n\
message Outer {\n\
  message Inner { string x = 1; }\n\
}";
        let proto = "syntax = \"proto3\";\n\
                      package test;\n\
                      import \"outer.proto\";\n\
                      message Msg {\n\
                        outer_pkg.Outer.Inner nested = 1;\n\
                      }";
        let result = resolve(proto, &[("outer.proto", imported_with_nested)]);
        assert!(
            result.contains("Outer.Inner nested = 1;"),
            "nested type should have package prefix stripped, got:\n{result}"
        );
        assert!(
            !result.contains("outer_pkg.Outer.Inner"),
            "package prefix should be removed"
        );
    }

    #[test]
    fn map_type_in_comment_not_replaced() {
        let proto = "syntax = \"proto3\";\n\
                      package test;\n\
                      import \"llama_cpp/media_input.proto\";\n\
                      message Msg {\n\
                        // map<string, llama_cpp.MediaInput> is not used here\n\
                        map<string, llama_cpp.MediaInput> items = 1;\n\
                      }";
        let result = resolve(proto, &[("llama_cpp/media_input.proto", IMPORTED)]);
        assert!(
            result.contains("// map<string, llama_cpp.MediaInput>"),
            "comment should not be modified, got:\n{result}"
        );
        assert!(
            result.contains("map<string, MediaInput>"),
            "code should have prefix stripped, got:\n{result}"
        );
    }

    #[test]
    fn rpc_type_in_trailing_comment_not_replaced() {
        let proto = "syntax = \"proto3\";\n\
                      package svc;\n\
                      import \"llama_cpp/media_input.proto\";\n\
                      service Svc {\n\
                        rpc Do(llama_cpp.MediaInput) returns (llama_cpp.MediaInput); // takes (llama_cpp.MediaInput)\n\
                      }";
        let result = resolve(proto, &[("llama_cpp/media_input.proto", IMPORTED)]);
        assert!(
            result.contains("rpc Do(MediaInput) returns (MediaInput);"),
            "rpc types should have prefix stripped, got:\n{result}"
        );
        assert!(
            result.contains("// takes (llama_cpp.MediaInput)"),
            "trailing comment should not be modified, got:\n{result}"
        );
    }

    #[test]
    fn import_public_resolved() {
        let proto = "syntax = \"proto3\";\n\
                      package test;\n\
                      import public \"llama_cpp/media_input.proto\";\n\
                      message Msg {\n\
                        llama_cpp.MediaInput m = 1;\n\
                      }";
        let result = resolve(proto, &[("llama_cpp/media_input.proto", IMPORTED)]);
        assert!(
            no_import_statement(&result),
            "import public should be resolved, got:\n{result}"
        );
        assert!(
            result.contains("MediaInput m = 1;"),
            "type ref should be resolved, got:\n{result}"
        );
    }

    #[test]
    fn escaped_quote_in_string_not_confused() {
        let proto = "syntax = \"proto3\";\n\
                      package test;\n\
                      import \"llama_cpp/media_input.proto\";\n\
                      message Msg {\n\
                        llama_cpp.MediaInput m = 1;\n\
                        string s = 2 [default = \"val\\\"with//slash\"];\n\
                      }";
        let result = resolve(proto, &[("llama_cpp/media_input.proto", IMPORTED)]);
        assert!(
            result.contains("MediaInput m = 1;"),
            "type ref should still be resolved, got:\n{result}"
        );
        assert!(
            result.contains(r#"[default = "val\"with//slash"]"#),
            "escaped-quote string should be preserved, got:\n{result}"
        );
    }
}
