// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Architecture guards over the source tree.
//!
//! Two invariants are checked by scanning the files under `src/` (no
//! `include_str!` lists to maintain; files are discovered on disk):
//!
//! 1. `src/domain` never depends on Ratatui (styles live in
//!    `crate::screens::components::theme`).
//! 2. Shared visual glyphs are never spelled out in screens: they come from
//!    `crate::screens::components::glyphs`.
//!
//! The scan is textual. Before matching, the scanner drops comment lines and
//! `#[cfg(test)] mod ... { }` blocks, because neither draws anything.
//!
//! # Adding a new screen
//!
//! Put the file under `src/screens/` (or a subfolder). It is picked up
//! automatically by the glyph guard. Do not hardcode reserved glyphs in it;
//! add a field to `Glyphs` instead. If the screen's tests need helper glyphs,
//! keep them inside its `#[cfg(test)] mod tests`.
//!
//! # Adding a reserved glyph
//!
//! Add the character to `RESERVED_GLYPHS` below and to the `Glyphs` set in
//! `src/screens/components/glyphs.rs`. Use only single-column glyphs (see the
//! ASCII fallback notes in that module).
//!
//! # Exemptions
//!
//! `GLYPH_GUARD_EXEMPT` lists the files allowed to contain glyphs: the owner
//! of the shared set and test-only helpers. Keep it short.
//!
//! # Known limitations
//!
//! - Text search, not parsing. A string literal that mentions `ratatui` or a
//!   glyph is flagged even though it draws nothing (a false positive). Rewrite
//!   the literal if this happens.
//! - Brace matching ignores braces inside string and char literals.
//! - Only `#[cfg(test)] mod` blocks are stripped. Other `cfg(test)` items are
//!   checked as production code.

use std::fs;
use std::path::{Path, PathBuf};

/// Glyphs owned by the shared glyph set.
const RESERVED_GLYPHS: &[char] = &[
    '○', '◐', '✔', '▲', '●', '▼', '▸', '⚠', '█', '░', '▌', '▏', '■', '□', '◆', '◇', '▪', '▫', '◌',
    '∅',
];

/// Files under `src/screens` that may contain glyphs (paths relative to `src/`).
const GLYPH_GUARD_EXEMPT: &[&str] = &[
    "screens/components/glyphs.rs",
    "screens/test_utils.rs",
    "screens/task_list_view/tests.rs",
];

fn src_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// All `.rs` files under `dir`, recursively, sorted.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).expect("directory should be readable") {
            let path = entry.expect("directory entry should be readable").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Path of `file` relative to `src/`, with `/` separators.
fn relative_to_src(file: &Path) -> String {
    file.strip_prefix(src_dir())
        .expect("file should live under src/")
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Source with comment lines and `#[cfg(test)] mod` blocks removed.
fn production_code(source: &str) -> String {
    let without_comments = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    strip_test_modules(&without_comments)
}

const CFG_TEST: &str = "#[cfg(test)]";

fn strip_test_modules(source: &str) -> String {
    let mut out = String::new();
    let mut rest = source;
    while let Some(idx) = rest.find(CFG_TEST) {
        let after = &rest[idx + CFG_TEST.len()..];
        if !after.trim_start().starts_with("mod ") {
            // Not a test module: keep it and continue after the attribute.
            out.push_str(&rest[..idx + CFG_TEST.len()]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..idx]);
        rest = match after.find('{') {
            Some(open) => &after[open + block_len(&after[open..])..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Length in bytes of the `{ ... }` block at the start of `s`, braces included.
fn block_len(s: &str) -> usize {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return i + c.len_utf8();
                }
            }
            _ => {}
        }
    }
    s.len()
}

fn domain_depends_on_ratatui(source: &str) -> bool {
    production_code(source).contains("ratatui")
}

fn hardcoded_glyphs(source: &str) -> Vec<char> {
    let code = production_code(source);
    RESERVED_GLYPHS
        .iter()
        .copied()
        .filter(|glyph| code.contains(*glyph))
        .collect()
}

#[test]
fn domain_does_not_depend_on_ratatui() {
    let files = rust_files(&src_dir().join("domain"));
    assert!(files.len() >= 5, "domain discovery looks broken: {files:?}");

    let offenders: Vec<String> = files
        .iter()
        .filter(|file| domain_depends_on_ratatui(&fs::read_to_string(file).unwrap()))
        .map(|file| relative_to_src(file))
        .collect();

    assert!(
        offenders.is_empty(),
        "domain files must not reference ratatui: found it in {offenders:?}"
    );
}

#[test]
fn screens_do_not_hardcode_glyphs() {
    let files = rust_files(&src_dir().join("screens"));
    assert!(
        files.len() >= 10,
        "screens discovery looks broken: {files:?}"
    );

    let offenders: Vec<String> = files
        .iter()
        .filter(|file| !GLYPH_GUARD_EXEMPT.contains(&relative_to_src(file).as_str()))
        .filter_map(|file| {
            let glyphs = hardcoded_glyphs(&fs::read_to_string(file).unwrap());
            (!glyphs.is_empty()).then(|| format!("{} {glyphs:?}", relative_to_src(file)))
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "these glyphs must come from the shared glyph set: {offenders:#?}"
    );
}

// Self-tests: the guards must catch violations and ignore what they should.

#[test]
fn ratatui_guard_flags_a_violation() {
    let source = "use ratatui::style::Style;\npub fn f() {}\n";
    assert!(domain_depends_on_ratatui(source));
}

#[test]
fn ratatui_guard_ignores_comments_and_test_modules() {
    let source = "// ratatui is not used here\npub fn f() {}\n\
                  #[cfg(test)]\nmod tests {\n    use ratatui::style::Style;\n}\n";
    assert!(!domain_depends_on_ratatui(source));
}

#[test]
fn glyph_guard_flags_a_hardcoded_glyph() {
    let source = "let icon = \"○ Todo\";\nlet bar = \"█░\";\n";
    assert_eq!(hardcoded_glyphs(source), vec!['○', '█', '░']);
}

#[test]
fn glyph_guard_ignores_comments_and_test_modules() {
    let source = "/// `○ Todo` in a doc comment\n\
                  // ▲ in a line comment\n\
                  pub fn f() {}\n\
                  #[cfg(test)]\nmod tests {\n    fn t() { assert!(s.contains('█')); }\n}\n";
    assert!(hardcoded_glyphs(source).is_empty());
}

#[test]
fn glyph_guard_still_checks_code_after_a_cfg_test_item() {
    // Only test *modules* are stripped; a `cfg(test)` function is production
    // code for this scanner, so the glyph after it must be flagged.
    let source = "#[cfg(test)]\nfn helper() {}\nlet icon = \"▲\";\n";
    assert_eq!(hardcoded_glyphs(source), vec!['▲']);
}

#[test]
fn glyph_guard_flags_string_literals_known_limitation() {
    // Known false positive: text search cannot tell a literal from a drawn glyph.
    let source = "let help = \"press ◐ to continue\";\n";
    assert_eq!(hardcoded_glyphs(source), vec!['◐']);
}

#[test]
fn strip_test_modules_handles_nested_braces() {
    let source = "a\n#[cfg(test)]\nmod tests { fn t() { if true { } } }\nb\n";
    let code = strip_test_modules(source);
    assert!(code.contains('a') && code.contains('b'));
    assert!(!code.contains("fn t"));
}
