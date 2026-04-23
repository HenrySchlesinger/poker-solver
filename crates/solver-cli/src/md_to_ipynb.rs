//! `md-to-ipynb` subcommand: convert our `colab/*.md` plan files into real
//! Jupyter `.ipynb` notebooks.
//!
//! ## Why this exists (Rust, not Python)
//!
//! The project rule (root `CLAUDE.md` §"Rust wherever possible") says dev
//! tools live in `solver-cli`, not stray Python. Notebook generation is a
//! parse-and-emit problem; nothing about it actually benefits from Python.
//! We emit nbformat v4.5 JSON directly via `serde_json` and save a Python
//! dependency (`jupytext`) that would otherwise need to be kept in sync
//! across every Colab session.
//!
//! ## Input: our `.md` plan format
//!
//! The Colab plans under `colab/` use a simple convention:
//!
//! ```text
//! # Some top-level title
//!
//! Some free-form intro text that becomes the first markdown cell.
//!
//! ## Cell 1: setup
//!
//! ```python
//! !rustc --version
//! ```
//!
//! ## Cell 2: do the thing
//!
//! ```bash
//! cargo build --release
//! ```
//!
//! ## Some trailing prose
//! ```
//!
//! A `## Cell N: <title>` heading marks the start of a code cell. The
//! fenced code block that follows (```python, ```bash, ```) becomes the
//! cell source. Any narrative between cells becomes a markdown cell.
//! Anything before the first `## Cell` header becomes a leading markdown
//! cell (title + intro). Anything after the last code fence becomes a
//! trailing markdown cell (the "Expected runtime" / "Coordination"
//! sections of our plans).
//!
//! ## Output: nbformat v4.5
//!
//! Spec: <https://nbformat.readthedocs.io/en/latest/format_description.html>
//!
//! Minimal top-level shape:
//!
//! ```json
//! {
//!   "cells": [...],
//!   "metadata": {
//!     "kernelspec": { "display_name": "Python 3", "language": "python",
//!                      "name": "python3" },
//!     "language_info": { "name": "python" }
//!   },
//!   "nbformat": 4,
//!   "nbformat_minor": 5
//! }
//! ```
//!
//! Each cell is either:
//!
//! - `{"cell_type": "markdown", "id": "...", "metadata": {}, "source": [...]}`
//! - `{"cell_type": "code", "id": "...", "metadata": {}, "source": [...],
//!     "execution_count": null, "outputs": []}`
//!
//! `source` is a list of strings that, when concatenated, reproduce the
//! cell contents. Each line (except the last) ends with `\n`.
//!
//! Cell `id` is required in v4.5. We use a deterministic scheme based on
//! the cell index so round-tripping is stable: `cell-<kind>-<N>`. This
//! matters because the notebooks are committed to git — a non-deterministic
//! UUID would make every regeneration produce a noisy diff.
//!
//! ## Non-goals
//!
//! - We do **not** round-trip `.ipynb` → `.md`. The markdown plans are the
//!   source of truth; notebooks are shipped artifacts.
//! - We do **not** execute cells or embed outputs. All cells emit with
//!   `"outputs": []` and `"execution_count": null`.

use std::io::Write;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

/// Arguments for `solver-cli md-to-ipynb`.
///
/// Kept separate from the `clap` struct in `main.rs` so the converter
/// itself is unit-testable without going through argument parsing.
#[derive(Debug)]
pub struct MdToIpynbArgs {
    /// Path to the input `.md` file.
    pub input: String,
    /// Path to write the `.ipynb` to. `-` writes to stdout.
    pub output: String,
}

/// One cell as we've parsed it out of the markdown plan, before JSON
/// emission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedCell {
    /// A markdown cell. `text` does NOT include a trailing newline; we
    /// add one at emit time so blank-line handling is consistent.
    Markdown { text: String },
    /// A code cell. `lang` is the fence language (e.g. `"python"`,
    /// `"bash"`); we preserve it in cell metadata so Colab can do syntax
    /// highlighting, but the Python 3 kernel handles `!...` shell escapes
    /// regardless.
    Code { lang: String, source: String },
}

/// End-to-end: read the input file, parse, emit JSON, write the output.
pub fn run_md_to_ipynb(args: &MdToIpynbArgs) -> Result<()> {
    let md = std::fs::read_to_string(&args.input)
        .with_context(|| format!("read input markdown {:?}", args.input))?;
    let cells = parse_markdown(&md).with_context(|| format!("parse markdown {:?}", args.input))?;
    let notebook = emit_notebook(&cells);

    // Pretty-printed. The file lives in git and will be read by humans
    // when diffing. The extra bytes are a non-issue.
    let text = serde_json::to_string_pretty(&notebook).context("serialize notebook JSON")?;

    write_output(&args.output, &text).with_context(|| format!("write output {:?}", args.output))?;
    Ok(())
}

fn write_output(path: &str, text: &str) -> Result<()> {
    if path == "-" {
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        lock.write_all(text.as_bytes())?;
        // Trailing newline — standard for text files.
        lock.write_all(b"\n")?;
        lock.flush()?;
        return Ok(());
    }
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create parent dir for {path:?}"))?;
        }
    }
    // Files end with a single trailing newline; many tools (git, editors)
    // expect this.
    let mut full = String::with_capacity(text.len() + 1);
    full.push_str(text);
    full.push('\n');
    std::fs::write(path, full)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Where we are in the state machine.
///
/// Factored out so `parse_markdown` reads as a clean per-line loop, and
/// so each transition is a single, named event with obvious effects.
#[derive(Debug)]
enum Scope {
    /// Not inside a `## Cell` section. Everything buffered here becomes
    /// a markdown cell when we hit the next `## Cell` header or EOF.
    /// Includes fenced code blocks that were NOT introduced by a
    /// `## Cell` header — those round-trip literally as part of the
    /// markdown.
    Outside,
    /// Inside a `## Cell N: ...` section, before its code fence opens.
    /// Prose here becomes its own markdown cell.
    CellProse,
}

#[derive(Debug)]
enum Fence {
    /// Not currently inside a fenced code block.
    None,
    /// Inside a fenced block. `inside_cell` is true iff the fence opened
    /// while we were in the `CellProse` scope — in which case its body
    /// will be emitted as a code cell. Otherwise the fence is transparent
    /// (its literal text is kept in the surrounding markdown buffer).
    Open {
        lang: String,
        inside_cell: bool,
        body: String,
    },
}

/// Parse a `.md` plan into an ordered list of cells.
///
/// Rules (matching our conventions in `colab/*.md`):
///
/// 1. Everything before the first `## Cell N:` header is one markdown
///    cell (title + intro). If there's no leading prose at all, no cell
///    is emitted.
/// 2. A `## Cell N: title` header starts a new section. The body of the
///    section is: (a) any prose between the header and the next fenced
///    code block (becomes a markdown cell if non-empty), then (b) the
///    fenced code block that follows (becomes a code cell).
/// 3. Anything after the last code fence, up to EOF (or the next
///    `## Cell` header), becomes a trailing markdown cell.
/// 4. Fenced code blocks NOT introduced by a `## Cell` header (the
///    trailing "On Henry's Mac: ```bash …```" examples in our plans)
///    round-trip literally in the surrounding markdown — we do NOT
///    spawn a separate code cell for them.
pub fn parse_markdown(md: &str) -> Result<Vec<ParsedCell>> {
    let mut cells: Vec<ParsedCell> = Vec::new();

    // Buffered markdown prose for the current scope. Flushed on scope
    // transitions and at EOF.
    let mut md_buf = String::new();
    let mut scope = Scope::Outside;
    let mut fence = Fence::None;

    for raw_line in md.split_inclusive('\n') {
        // Strip trailing \n (if any — last line may not have one) for
        // classification. We write the original `raw_line` to buffers
        // so line endings round-trip exactly.
        let stripped = raw_line.trim_end_matches('\n');

        // Classify what kind of line this is. We do this up front so the
        // dispatch below is obvious.
        let line_kind = classify_line(stripped);

        match (&mut fence, &scope, line_kind) {
            // -------- Inside a fenced code block --------
            //
            // Fences are closed by a bare "```" line. Everything else
            // (including nested-looking "```python" lines — which
            // shouldn't happen in well-formed markdown, but we handle
            // them as content) accumulates into the body.
            (
                Fence::Open {
                    lang,
                    inside_cell,
                    body,
                },
                _,
                LineKind::FenceClose,
            ) => {
                let lang_s = lang.clone();
                let was_inside = *inside_cell;
                let body_s = std::mem::take(body);
                fence = Fence::None;
                if was_inside {
                    // Emit the code cell. Strip the final trailing
                    // newline we accumulated from the last content line;
                    // we add newlines back when emitting source lines.
                    cells.push(ParsedCell::Code {
                        lang: lang_s,
                        source: body_s.trim_end_matches('\n').to_string(),
                    });
                    // A `## Cell` section ends when its code fence
                    // closes — anything after is either trailing prose
                    // or the next `## Cell` header, not a continuation.
                    // Drop back to Outside explicitly here so the
                    // fence-open arm above doesn't re-enter CellProse
                    // semantics on a later unrelated fence.
                    scope = Scope::Outside;
                } else {
                    // Fence was in markdown prose — pass it through
                    // literally. Reconstruct the fence delimiters so
                    // the markdown cell reproduces the original text.
                    md_buf.push_str("```");
                    md_buf.push_str(&lang_s);
                    md_buf.push('\n');
                    md_buf.push_str(&body_s);
                    md_buf.push_str("```\n");
                }
            }
            (Fence::Open { body, .. }, _, _) => {
                // Any other line inside the fence is content. Preserve
                // the original line ending verbatim.
                body.push_str(raw_line);
            }

            // -------- Not inside a fenced code block --------
            //
            // Two things can change state here: a `## Cell` header, or
            // a fence-open line. Everything else appends to md_buf.
            (Fence::None, _, LineKind::CellHeader) => {
                // Flush whatever prose we've accumulated as a markdown
                // cell. Whether we're in Outside or CellProse doesn't
                // matter for the flush itself — both buffer prose into
                // the same `md_buf` and the same markdown cell shape.
                if let Some(cell) = flush_markdown(&md_buf) {
                    cells.push(cell);
                }
                md_buf.clear();
                scope = Scope::CellProse;
            }
            (Fence::None, Scope::CellProse, LineKind::FenceOpen(lang)) => {
                // Flush the cell's prose (if any) as a markdown cell,
                // then start capturing the fence body as a code cell.
                if let Some(cell) = flush_markdown(&md_buf) {
                    cells.push(cell);
                }
                md_buf.clear();
                fence = Fence::Open {
                    lang,
                    inside_cell: true,
                    body: String::new(),
                };
            }
            (Fence::None, Scope::Outside, LineKind::FenceOpen(lang)) => {
                // Fence opened in plain prose — capture the body but
                // mark it `inside_cell: false` so it round-trips as
                // literal markdown when the fence closes.
                fence = Fence::Open {
                    lang,
                    inside_cell: false,
                    body: String::new(),
                };
            }
            (Fence::None, _, LineKind::Other) => {
                md_buf.push_str(raw_line);
            }
            (Fence::None, _, LineKind::FenceClose) => {
                // A bare "```" line outside any open fence is unusual —
                // in practice it only happens if the markdown is
                // malformed. Treat as literal markdown content rather
                // than erroring; the notebook will still be valid.
                md_buf.push_str(raw_line);
            }
        }
    }

    // EOF handling.
    if let Fence::Open { .. } = fence {
        bail!("unclosed code fence at end of file");
    }
    if let Some(cell) = flush_markdown(&md_buf) {
        cells.push(cell);
    }

    Ok(cells)
}

/// Line classification used by the parse loop.
#[derive(Debug)]
enum LineKind {
    /// `## Cell N: ...` header.
    CellHeader,
    /// Opening line of a fenced code block: ```` ```lang ````.
    FenceOpen(String),
    /// Bare closing fence: ```` ``` ````.
    FenceClose,
    /// Everything else (plain prose, other headers, blank lines, …).
    Other,
}

fn classify_line(stripped: &str) -> LineKind {
    if stripped.starts_with("## Cell ") {
        return LineKind::CellHeader;
    }
    let trimmed = stripped.trim();
    if trimmed == "```" {
        return LineKind::FenceClose;
    }
    if let Some(rest) = trimmed.strip_prefix("```") {
        // ```<lang> [rest]
        let lang = rest.split_whitespace().next().unwrap_or("");
        if !lang.is_empty() {
            return LineKind::FenceOpen(lang.to_string());
        }
        // Exactly "```" with trailing whitespace? Treat as close.
        return LineKind::FenceClose;
    }
    LineKind::Other
}

/// Build a markdown `ParsedCell` from buffered text. Returns `None` if
/// the text is empty (or only blank lines) — we don't emit empty
/// markdown cells.
fn flush_markdown(buf: &str) -> Option<ParsedCell> {
    let trimmed = buf.trim_matches('\n');
    if trimmed.trim().is_empty() {
        return None;
    }
    Some(ParsedCell::Markdown {
        text: trimmed.to_string(),
    })
}

/// Strip the leading `## Cell N: ` prefix from a title and, if it was
/// present, return the tail. Used only for tests / potential metadata
/// annotation.
#[allow(dead_code)]
pub fn cell_header_title(line: &str) -> Option<&str> {
    let tail = line.strip_prefix("## Cell ")?;
    // Expect `N: rest`
    let (_, rest) = tail.split_once(':')?;
    Some(rest.trim())
}

// ---------------------------------------------------------------------------
// Emission: nbformat v4.5
// ---------------------------------------------------------------------------

/// Turn a list of parsed cells into a full nbformat v4.5 notebook.
pub fn emit_notebook(cells: &[ParsedCell]) -> Value {
    let emitted: Vec<Value> = cells
        .iter()
        .enumerate()
        .map(|(i, cell)| emit_cell(i, cell))
        .collect();

    json!({
        "cells": emitted,
        "metadata": {
            "kernelspec": {
                "display_name": "Python 3",
                "language": "python",
                "name": "python3"
            },
            "language_info": {
                "name": "python"
            },
            "colab": {
                "provenance": []
            }
        },
        "nbformat": 4,
        "nbformat_minor": 5
    })
}

/// Emit one cell. See module-level docs for the shape.
fn emit_cell(index: usize, cell: &ParsedCell) -> Value {
    match cell {
        ParsedCell::Markdown { text } => {
            let id = format!("cell-md-{index:03}");
            let source = text_to_source_lines(text);
            json!({
                "cell_type": "markdown",
                "id": id,
                "metadata": {},
                "source": source
            })
        }
        ParsedCell::Code { lang, source } => {
            let id = format!("cell-code-{index:03}");
            let src = text_to_source_lines(source);
            // nbformat requires code cells to have execution_count and
            // outputs fields; they can be null / empty respectively.
            //
            // Cell-level metadata carries the fence language so Colab
            // keeps it associated even if the kernel is Python.
            let mut meta = Map::new();
            meta.insert("source_language".to_string(), Value::String(lang.clone()));
            json!({
                "cell_type": "code",
                "id": id,
                "metadata": meta,
                "execution_count": Value::Null,
                "outputs": [],
                "source": src
            })
        }
    }
}

/// Split multi-line text into the array-of-strings form nbformat uses
/// for cell `source`. Each element except the last ends with `\n`.
/// An empty input produces an empty array.
fn text_to_source_lines(text: &str) -> Vec<Value> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<Value> = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'\n' {
            // Include the newline in the emitted string, per nbformat
            // convention.
            let segment = &text[start..=i];
            out.push(Value::String(segment.to_string()));
            start = i + 1;
        }
    }
    // Trailing content without a final newline becomes the last element
    // without a `\n` suffix.
    if start < text.len() {
        out.push(Value::String(text[start..].to_string()));
    }
    out
}

// ---------------------------------------------------------------------------
// Schema sanity checks
// ---------------------------------------------------------------------------

/// Minimal nbformat v4.5 structural validation. Not a full JSON Schema
/// check — we validate the specific shape `emit_notebook` produces, so
/// a regression in emission is caught immediately. Used by the
/// integration tests and available for ad-hoc callers.
///
/// Returns `Ok(())` if the notebook looks structurally valid.
//
// `dead_code` suppressed: this is a public API used from the test module
// and available to integration tests / ad-hoc callers, but `main.rs` only
// needs `run_md_to_ipynb`. Keeping it public is deliberate.
#[allow(dead_code)]
pub fn validate_nbformat_v45(notebook: &Value) -> Result<()> {
    let root = notebook
        .as_object()
        .ok_or_else(|| anyhow!("notebook root is not an object"))?;

    let nbformat = root
        .get("nbformat")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("missing integer field 'nbformat'"))?;
    if nbformat != 4 {
        bail!("nbformat must be 4, got {nbformat}");
    }
    let nbformat_minor = root
        .get("nbformat_minor")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("missing integer field 'nbformat_minor'"))?;
    if nbformat_minor < 5 {
        bail!("nbformat_minor must be >= 5, got {nbformat_minor}");
    }

    let metadata = root
        .get("metadata")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("missing object field 'metadata'"))?;
    // kernelspec / language_info are recommended but not strictly
    // required; we check them because we always emit them.
    metadata
        .get("kernelspec")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("metadata.kernelspec must be an object"))?;
    metadata
        .get("language_info")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("metadata.language_info must be an object"))?;

    let cells = root
        .get("cells")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("missing array field 'cells'"))?;

    for (i, cell) in cells.iter().enumerate() {
        validate_cell(cell).with_context(|| format!("cell {i}"))?;
    }

    Ok(())
}

#[allow(dead_code)] // same rationale as `validate_nbformat_v45` above
fn validate_cell(cell: &Value) -> Result<()> {
    let obj = cell
        .as_object()
        .ok_or_else(|| anyhow!("cell is not an object"))?;
    let cell_type = obj
        .get("cell_type")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("cell missing 'cell_type'"))?;

    // In v4.5, every cell needs an id.
    obj.get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("cell missing 'id' (required in v4.5)"))?;

    obj.get("metadata")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("cell missing 'metadata' object"))?;

    let source = obj
        .get("source")
        .ok_or_else(|| anyhow!("cell missing 'source'"))?;
    // Source may be a string or a list of strings. We only emit lists.
    match source {
        Value::Array(arr) => {
            for (j, s) in arr.iter().enumerate() {
                if !s.is_string() {
                    bail!("cell source[{j}] is not a string");
                }
            }
        }
        Value::String(_) => {}
        _ => bail!("cell 'source' must be a string or array of strings"),
    }

    match cell_type {
        "markdown" => {}
        "code" => {
            // execution_count required (null allowed), outputs required.
            if !obj.contains_key("execution_count") {
                bail!("code cell missing 'execution_count'");
            }
            let outputs = obj
                .get("outputs")
                .ok_or_else(|| anyhow!("code cell missing 'outputs'"))?;
            if !outputs.is_array() {
                bail!("code cell 'outputs' must be an array");
            }
        }
        "raw" => {}
        other => bail!("unknown cell_type {:?}", other),
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_line_smoke() {
        assert!(matches!(
            classify_line("## Cell 1: setup"),
            LineKind::CellHeader
        ));
        assert!(matches!(classify_line("```python"), LineKind::FenceOpen(ref l) if l == "python"));
        assert!(matches!(classify_line("```bash"), LineKind::FenceOpen(ref l) if l == "bash"));
        assert!(matches!(classify_line("```"), LineKind::FenceClose));
        assert!(matches!(classify_line("plain text"), LineKind::Other));
        assert!(matches!(
            classify_line("# Not a cell header"),
            LineKind::Other
        ));
        assert!(matches!(
            classify_line("## Some non-cell header"),
            LineKind::Other
        ));
    }

    #[test]
    fn cell_header_title_parses() {
        assert_eq!(cell_header_title("## Cell 1: setup"), Some("setup"));
        assert_eq!(
            cell_header_title("## Cell 42: do all the things"),
            Some("do all the things")
        );
        assert_eq!(cell_header_title("## Coordination"), None);
        assert_eq!(cell_header_title("# Title"), None);
    }

    #[test]
    fn parse_single_code_cell() {
        let md = "# Title\n\n\
                  Intro prose.\n\n\
                  ## Cell 1: setup\n\n\
                  ```python\n\
                  import os\n\
                  print('hi')\n\
                  ```\n";
        let cells = parse_markdown(md).unwrap();
        assert_eq!(cells.len(), 2, "got: {cells:#?}");
        match &cells[0] {
            ParsedCell::Markdown { text } => {
                assert!(text.contains("Title"));
                assert!(text.contains("Intro prose"));
            }
            _ => panic!("expected first cell markdown"),
        }
        match &cells[1] {
            ParsedCell::Code { lang, source } => {
                assert_eq!(lang, "python");
                assert!(source.contains("import os"));
                assert!(source.contains("print('hi')"));
                // No trailing newline (we trim at emission).
                assert!(!source.ends_with('\n'));
            }
            _ => panic!("expected second cell code"),
        }
    }

    #[test]
    fn parse_multiple_cells_with_prose_between() {
        let md = "\
# Title

Leading.

## Cell 1: setup

Some prose before the fence.

```python
a = 1
```

## Cell 2: use

```python
print(a)
```

## Epilogue

Trailing text.
";
        let cells = parse_markdown(md).unwrap();
        // Expect: leading md, cell1 prose md, cell1 code, cell2 code,
        // trailing md.
        assert!(cells.len() >= 4, "got {} cells: {cells:#?}", cells.len());
        // First cell should be the title+leading markdown.
        match &cells[0] {
            ParsedCell::Markdown { text } => assert!(text.contains("Title")),
            _ => panic!("wrong first cell"),
        }
        // Must contain both code cells.
        let code_cells: Vec<_> = cells
            .iter()
            .filter_map(|c| match c {
                ParsedCell::Code { source, .. } => Some(source.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(code_cells.len(), 2, "cells: {cells:#?}");
        assert!(code_cells[0].contains("a = 1"));
        assert!(code_cells[1].contains("print(a)"));
        // Trailing "## Epilogue" should end up in a markdown cell.
        let has_epilogue = cells.iter().any(|c| match c {
            ParsedCell::Markdown { text } => text.contains("Epilogue"),
            _ => false,
        });
        assert!(has_epilogue, "lost the trailing Epilogue section");
    }

    #[test]
    fn parse_bash_cell() {
        let md = "## Cell 1: run\n\n\
                  ```bash\n\
                  echo hi\n\
                  ```\n";
        let cells = parse_markdown(md).unwrap();
        let codes: Vec<_> = cells
            .iter()
            .filter_map(|c| match c {
                ParsedCell::Code { lang, source } => Some((lang.clone(), source.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].0, "bash");
        assert!(codes[0].1.contains("echo hi"));
    }

    #[test]
    fn parse_rejects_unclosed_fence() {
        let md = "## Cell 1: bad\n\n\
                  ```python\n\
                  never_closed = True\n";
        let err = parse_markdown(md).unwrap_err();
        assert!(err.to_string().contains("unclosed"), "got: {err}");
    }

    #[test]
    fn trailing_bash_fence_stays_in_markdown() {
        // Matches the shape of `precompute_preflop.md`'s trailing
        // "On Henry's Mac: ```bash ... ```" example — that fence is
        // inside plain prose, NOT under a `## Cell` header.
        let md = "\
## Cell 1: setup

```python
print('hi')
```

## Download locally

```bash
rclone copy foo
```

## Notes

A trailing paragraph.
";
        let cells = parse_markdown(md).unwrap();
        // Exactly one code cell (the `## Cell 1` one). The trailing
        // bash fence lives inside the trailing markdown cell.
        let code_count = cells
            .iter()
            .filter(|c| matches!(c, ParsedCell::Code { .. }))
            .count();
        assert_eq!(code_count, 1, "cells: {cells:#?}");
        let tail_md = cells
            .iter()
            .filter_map(|c| match c {
                ParsedCell::Markdown { text } => Some(text.as_str()),
                _ => None,
            })
            .last()
            .unwrap();
        assert!(tail_md.contains("rclone copy foo"), "tail: {tail_md}");
        assert!(tail_md.contains("A trailing paragraph"), "tail: {tail_md}");
        assert!(tail_md.contains("```bash"), "tail: {tail_md}");
    }

    #[test]
    fn emit_notebook_shape_is_valid_v45() {
        let cells = vec![
            ParsedCell::Markdown {
                text: "# Title".to_string(),
            },
            ParsedCell::Code {
                lang: "python".to_string(),
                source: "print('hi')".to_string(),
            },
        ];
        let nb = emit_notebook(&cells);
        // Spot-check the expected shape.
        assert_eq!(nb["nbformat"], 4);
        assert_eq!(nb["nbformat_minor"], 5);
        assert!(nb["metadata"]["kernelspec"].is_object());
        let cells_arr = nb["cells"].as_array().unwrap();
        assert_eq!(cells_arr.len(), 2);
        assert_eq!(cells_arr[0]["cell_type"], "markdown");
        assert_eq!(cells_arr[1]["cell_type"], "code");
        // Both cells have ids.
        assert!(cells_arr[0]["id"].is_string());
        assert!(cells_arr[1]["id"].is_string());
        // Code cell has execution_count + outputs.
        assert!(cells_arr[1]["execution_count"].is_null());
        assert!(cells_arr[1]["outputs"].is_array());
        // Validator accepts it.
        validate_nbformat_v45(&nb).unwrap();
    }

    #[test]
    fn text_to_source_lines_splits_correctly() {
        assert_eq!(text_to_source_lines(""), Vec::<Value>::new());
        assert_eq!(
            text_to_source_lines("one"),
            vec![Value::String("one".to_string())]
        );
        assert_eq!(
            text_to_source_lines("one\ntwo"),
            vec![
                Value::String("one\n".to_string()),
                Value::String("two".to_string()),
            ]
        );
        assert_eq!(
            text_to_source_lines("one\ntwo\n"),
            vec![
                Value::String("one\n".to_string()),
                Value::String("two\n".to_string()),
            ]
        );
    }

    #[test]
    fn validate_rejects_bad_nbformat_version() {
        let bad = json!({
            "nbformat": 3,
            "nbformat_minor": 0,
            "metadata": { "kernelspec": {}, "language_info": {} },
            "cells": []
        });
        let err = validate_nbformat_v45(&bad).unwrap_err();
        assert!(err.to_string().contains("nbformat must be 4"));
    }

    /// Stringify an anyhow error chain so we can assert on the inner
    /// source message (anyhow's `Display` only shows the outermost
    /// context by default).
    fn full_err_chain(err: &anyhow::Error) -> String {
        err.chain()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join(" / ")
    }

    #[test]
    fn validate_rejects_missing_cell_id() {
        let bad = json!({
            "nbformat": 4,
            "nbformat_minor": 5,
            "metadata": { "kernelspec": {}, "language_info": {} },
            "cells": [{
                "cell_type": "markdown",
                "metadata": {},
                "source": []
            }]
        });
        let err = validate_nbformat_v45(&bad).unwrap_err();
        let chain = full_err_chain(&err);
        assert!(chain.contains("id"), "got: {chain}");
    }

    #[test]
    fn validate_rejects_code_cell_missing_outputs() {
        let bad = json!({
            "nbformat": 4,
            "nbformat_minor": 5,
            "metadata": { "kernelspec": {}, "language_info": {} },
            "cells": [{
                "cell_type": "code",
                "id": "cell-code-0",
                "metadata": {},
                "source": [],
                "execution_count": null
            }]
        });
        let err = validate_nbformat_v45(&bad).unwrap_err();
        let chain = full_err_chain(&err);
        assert!(chain.contains("outputs"), "got: {chain}");
    }

    #[test]
    fn round_trip_our_actual_plan_shape() {
        // Mimic the structure of `colab/precompute_preflop.md`.
        let md = "\
# Precompute preflop ranges (Colab notebook plan)

Target: one-time job.

## Cell 1: setup

```python
!curl https://sh.rustup.rs | sh
```

## Cell 2: build

```python
!cargo build --release
```

## Download locally

```bash
# On Henry's Mac:
rclone copy ...
```

## Expected runtime

~8 hours.
";
        let cells = parse_markdown(md).unwrap();
        let nb = emit_notebook(&cells);
        validate_nbformat_v45(&nb).unwrap();

        // Should have at least 2 code cells (the two inside `## Cell`
        // sections). The `## Download locally` bash block is NOT a
        // `## Cell` section, so its ```bash block stays inline in the
        // trailing markdown.
        let cells_arr = nb["cells"].as_array().unwrap();
        let code_count = cells_arr
            .iter()
            .filter(|c| c["cell_type"] == "code")
            .count();
        assert_eq!(code_count, 2, "cells: {cells_arr:#?}");

        // The trailing markdown cell should contain both the "Download"
        // prose AND the "Expected runtime" prose.
        let last = cells_arr.last().unwrap();
        assert_eq!(last["cell_type"], "markdown");
        let last_src = last["source"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join("");
        assert!(
            last_src.contains("Expected runtime"),
            "missing Expected runtime: {last_src}"
        );
    }
}
