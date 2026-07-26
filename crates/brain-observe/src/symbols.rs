//! Lightweight, line-based symbol and import extraction.
//!
//! Deliberately not tree-sitter: the twin's promise at v1 is *orientation*,
//! not compiler-grade analysis. Each extractor scans trimmed line prefixes,
//! skips obvious comment lines, and degrades gracefully — an unknown
//! language still gets file-level twinning, just no symbols. Precision
//! limits are documented in docs/twin.md.

#[derive(Debug, Clone, PartialEq)]
pub struct Symbol {
    pub kind: &'static str,
    pub name: String,
    pub line: usize,
}

#[derive(Debug, Default, PartialEq)]
pub struct FileStructure {
    pub language: &'static str,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<String>,
}

pub fn analyze(rel_path: &str, content: &str) -> FileStructure {
    let ext = rel_path.rsplit('.').next().unwrap_or("");
    match ext {
        "rs" => analyze_lines(content, "rust", rust_line),
        "php" => analyze_lines(content, "php", php_line),
        "py" => analyze_lines(content, "python", python_line),
        "js" | "jsx" | "ts" | "tsx" => analyze_lines(content, "javascript", js_line),
        _ => FileStructure::default(),
    }
}

enum Found {
    Symbol(&'static str, String),
    Import(String),
    Nothing,
}

fn analyze_lines(
    content: &str,
    language: &'static str,
    classify: fn(&str) -> Found,
) -> FileStructure {
    let mut out = FileStructure { language, ..Default::default() };
    for (i, raw) in content.lines().enumerate() {
        let line = raw.trim();
        // '#' lines (Python/PHP comments, Rust attributes) are handled by the
        // per-language classifiers, where '#' never introduces a symbol.
        if line.is_empty()
            || line.starts_with("//")
            || line.starts_with('*')
            || line.starts_with("/*")
        {
            continue;
        }
        match classify(line) {
            Found::Symbol(kind, name) if !name.is_empty() => {
                out.symbols.push(Symbol { kind, name, line: i + 1 });
            }
            Found::Import(path) if !path.is_empty() => {
                if !out.imports.contains(&path) {
                    out.imports.push(path);
                }
            }
            _ => {}
        }
    }
    out
}

/// First identifier ([A-Za-z0-9_] run) at the start of `s`.
fn ident(s: &str) -> String {
    s.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

fn strip_any<'a>(mut s: &'a str, prefixes: &[&str]) -> &'a str {
    loop {
        let mut stripped = false;
        for p in prefixes {
            if let Some(rest) = s.strip_prefix(p) {
                s = rest;
                stripped = true;
            }
        }
        if !stripped {
            return s;
        }
    }
}

fn rust_line(line: &str) -> Found {
    let l = strip_any(line, &["pub(crate) ", "pub(super) ", "pub ", "async ", "unsafe ", "const "]);
    for (kw, kind) in [
        ("fn ", "fn"),
        ("struct ", "struct"),
        ("enum ", "enum"),
        ("trait ", "trait"),
        ("mod ", "mod"),
    ] {
        if let Some(rest) = l.strip_prefix(kw) {
            return Found::Symbol(kind, ident(rest));
        }
    }
    if let Some(rest) = line.strip_prefix("use ") {
        let path = rest.trim_end_matches(';').trim();
        // Drop brace-groups: `use a::b::{c, d}` -> `a::b`
        let path = path.split("::{").next().unwrap_or(path).trim_end_matches("::");
        return Found::Import(path.to_string());
    }
    Found::Nothing
}

fn php_line(line: &str) -> Found {
    if line.starts_with('#') {
        return Found::Nothing;
    }
    let l = strip_any(
        line,
        &["public ", "private ", "protected ", "static ", "final ", "abstract "],
    );
    for (kw, kind) in [
        ("class ", "class"),
        ("interface ", "interface"),
        ("trait ", "trait"),
        ("function ", "function"),
    ] {
        if let Some(rest) = l.strip_prefix(kw) {
            return Found::Symbol(kind, ident(rest));
        }
    }
    if let Some(rest) = line.strip_prefix("namespace ") {
        return Found::Symbol("namespace", rest.trim_end_matches(';').trim().to_string());
    }
    if let Some(rest) = line.strip_prefix("use ") {
        // `use X\Y as Z;` -> X\Y ; ignores closure `use ($x)` since that
        // never starts a line in idiomatic code.
        let path = rest.trim_end_matches(';').trim();
        let path = path.split(" as ").next().unwrap_or(path).trim();
        if path.starts_with('(') {
            return Found::Nothing;
        }
        return Found::Import(path.to_string());
    }
    Found::Nothing
}

fn python_line(line: &str) -> Found {
    if line.starts_with('#') {
        return Found::Nothing;
    }
    let l = strip_any(line, &["async "]);
    if let Some(rest) = l.strip_prefix("def ") {
        return Found::Symbol("function", ident(rest));
    }
    if let Some(rest) = l.strip_prefix("class ") {
        return Found::Symbol("class", ident(rest));
    }
    if let Some(rest) = line.strip_prefix("from ") {
        let module = rest.split_whitespace().next().unwrap_or("");
        return Found::Import(module.to_string());
    }
    if let Some(rest) = line.strip_prefix("import ") {
        // `import a, b as c` -> first module only (best-effort)
        let module = rest.split([',', ' ']).next().unwrap_or("");
        return Found::Import(module.to_string());
    }
    Found::Nothing
}

fn js_line(line: &str) -> Found {
    let l = strip_any(line, &["export default ", "export ", "async "]);
    if let Some(rest) = l.strip_prefix("function ") {
        return Found::Symbol("function", ident(rest));
    }
    if let Some(rest) = l.strip_prefix("class ") {
        return Found::Symbol("class", ident(rest));
    }
    if line.starts_with("import ") || line.starts_with("export ") {
        if let Some(path) = quoted_after(line, " from ") {
            return Found::Import(path);
        }
        if let Some(rest) = line.strip_prefix("import ") {
            // Bare side-effect import: `import './setup'`
            let rest = rest.trim().trim_end_matches(';');
            if (rest.starts_with('\'') || rest.starts_with('"')) && rest.len() >= 2 {
                return Found::Import(rest[1..rest.len() - 1].to_string());
            }
        }
    }
    if let Some(pos) = line.find("require(") {
        let rest = &line[pos + "require(".len()..];
        let rest = rest.trim_start();
        if let Some(q) = rest.chars().next() {
            if q == '\'' || q == '"' {
                if let Some(end) = rest[1..].find(q) {
                    return Found::Import(rest[1..1 + end].to_string());
                }
            }
        }
    }
    Found::Nothing
}

/// The quoted string following `marker`, e.g. `... from './util'` -> ./util
fn quoted_after(line: &str, marker: &str) -> Option<String> {
    let rest = &line[line.find(marker)? + marker.len()..];
    let rest = rest.trim_start();
    let quote = rest.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let end = rest[1..].find(quote)?;
    Some(rest[1..1 + end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(fs: &FileStructure, kind: &str) -> Vec<String> {
        fs.symbols
            .iter()
            .filter(|s| s.kind == kind)
            .map(|s| s.name.clone())
            .collect()
    }

    #[test]
    fn rust_symbols_and_imports() {
        let src = "use std::collections::{BTreeMap, BTreeSet};\n\
                   use crate::ids::NodeId;\n\
                   // fn commented_out()\n\
                   pub struct Store { }\n\
                   pub(crate) fn append_event() {}\n\
                   pub async fn fetch() {}\n\
                   enum Kind { A }\n\
                   pub trait Index {}\n\
                   mod tests;\n";
        let fs = analyze("src/lib.rs", src);
        assert_eq!(fs.language, "rust");
        assert_eq!(names(&fs, "struct"), vec!["Store"]);
        assert_eq!(names(&fs, "fn"), vec!["append_event", "fetch"]);
        assert_eq!(names(&fs, "enum"), vec!["Kind"]);
        assert_eq!(names(&fs, "trait"), vec!["Index"]);
        assert_eq!(names(&fs, "mod"), vec!["tests"]);
        assert_eq!(fs.imports, vec!["std::collections", "crate::ids::NodeId"]);
        assert_eq!(fs.symbols[0].line, 4, "line numbers are 1-based");
    }

    #[test]
    fn php_symbols_and_imports() {
        let src = "<?php\n\
                   namespace Pimcore\\Model;\n\
                   use Pimcore\\Db as Database;\n\
                   use Pimcore\\Cache;\n\
                   abstract class AbstractModel {\n\
                   public function getById($id) {}\n\
                   private static function helper() {}\n\
                   }\n\
                   interface Loader {}\n\
                   trait Cachable {}\n";
        let fs = analyze("src/Model.php", src);
        assert_eq!(fs.language, "php");
        assert_eq!(names(&fs, "class"), vec!["AbstractModel"]);
        assert_eq!(names(&fs, "function"), vec!["getById", "helper"]);
        assert_eq!(names(&fs, "interface"), vec!["Loader"]);
        assert_eq!(names(&fs, "trait"), vec!["Cachable"]);
        assert_eq!(names(&fs, "namespace"), vec!["Pimcore\\Model"]);
        assert_eq!(fs.imports, vec!["Pimcore\\Db", "Pimcore\\Cache"]);
    }

    #[test]
    fn python_symbols_and_imports() {
        let src = "import os\n\
                   import json, sys\n\
                   from pathlib import Path\n\
                   # def not_a_symbol():\n\
                   class Runner:\n\
                       def run(self):\n\
                           pass\n\
                   async def main():\n\
                       pass\n";
        let fs = analyze("run.py", src);
        assert_eq!(fs.language, "python");
        assert_eq!(names(&fs, "class"), vec!["Runner"]);
        assert_eq!(names(&fs, "function"), vec!["run", "main"]);
        assert_eq!(fs.imports, vec!["os", "json", "pathlib"]);
    }

    #[test]
    fn javascript_symbols_and_imports() {
        let src = "import React from 'react';\n\
                   import { useState } from \"react\";\n\
                   import './setup';\n\
                   const db = require('./db');\n\
                   export default function App() {}\n\
                   export class Store {}\n\
                   async function load() {}\n";
        let fs = analyze("src/app.tsx", src);
        assert_eq!(fs.language, "javascript");
        assert_eq!(names(&fs, "function"), vec!["App", "load"]);
        assert_eq!(names(&fs, "class"), vec!["Store"]);
        assert_eq!(fs.imports, vec!["react", "./setup", "./db"]);
    }

    #[test]
    fn unknown_language_falls_back_to_file_level() {
        let fs = analyze("Cargo.toml", "[package]\nname = \"brain\"\n");
        assert_eq!(fs, FileStructure::default());
    }
}
