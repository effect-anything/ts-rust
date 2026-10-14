//! Terminal output: colors, diagnostics with code frames, unified diffs.

use std::fmt::Write as _;
use std::io::IsTerminal;

/// `--color auto|always|never`. `auto` colors a terminal unless `NO_COLOR` is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

impl ColorChoice {
    /// # Errors
    /// Not one of `auto`, `always`, `never`.
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        match value {
            None | Some("auto") => Ok(Self::Auto),
            Some("always") => Ok(Self::Always),
            Some("never") => Ok(Self::Never),
            Some(other) => Err(format!("--color takes auto, always or never, not {other}")),
        }
    }

    #[must_use]
    pub fn stdout(self) -> Style {
        self.for_stream(std::io::stdout().is_terminal())
    }

    #[must_use]
    pub fn stderr(self) -> Style {
        self.for_stream(std::io::stderr().is_terminal())
    }

    fn for_stream(self, terminal: bool) -> Style {
        Style {
            on: match self {
                Self::Always => true,
                Self::Never => false,
                Self::Auto => terminal && std::env::var_os("NO_COLOR").is_none(),
            },
        }
    }
}

/// ANSI styling, or none.
#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub on: bool,
}

impl Style {
    fn paint(self, code: &str, text: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }
    #[must_use]
    pub fn red(self, text: &str) -> String {
        self.paint("1;31", text)
    }
    #[must_use]
    pub fn yellow(self, text: &str) -> String {
        self.paint("1;33", text)
    }
    #[must_use]
    pub fn green(self, text: &str) -> String {
        self.paint("32", text)
    }
    #[must_use]
    pub fn cyan(self, text: &str) -> String {
        self.paint("36", text)
    }
    #[must_use]
    pub fn blue(self, text: &str) -> String {
        self.paint("1;34", text)
    }
    #[must_use]
    pub fn bold(self, text: &str) -> String {
        self.paint("1", text)
    }
    #[must_use]
    pub fn dim(self, text: &str) -> String {
        self.paint("2", text)
    }
}

/// 1-based line and column (in characters) of byte `offset`.
#[must_use]
pub fn line_col(text: &str, offset: u32) -> (usize, usize) {
    let offset = usize::try_from(offset).unwrap_or(0).min(text.len());
    let mut offset = offset;
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    (line, column)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

/// A diagnostic in the author's file (byte offsets into `source`).
#[derive(Clone, Debug)]
pub struct Diag {
    pub severity: Severity,
    /// `SWR4001`, or empty.
    pub code: String,
    pub message: String,
    pub start: u32,
    pub end: u32,
    pub related: Vec<(String, u32, u32)>,
}

impl Diag {
    /// Split a `SWRnnnn: message` (or `TSnnnn: ...`) into code and text.
    #[must_use]
    pub fn new(
        severity: Severity,
        message: &str,
        start: u32,
        end: u32,
        related: Vec<(String, u32, u32)>,
    ) -> Self {
        let (code, message) = match message.split_once(": ") {
            Some((code, rest))
                if (code.starts_with("SWR") || code.starts_with("TS"))
                    && code.len() > 2
                    && code
                        .trim_start_matches(char::is_alphabetic)
                        .chars()
                        .all(|c| c.is_ascii_digit()) =>
            {
                (code.to_owned(), rest.to_owned())
            }
            _ => (String::new(), message.to_owned()),
        };
        Self {
            severity,
            code,
            message,
            start,
            end,
            related,
        }
    }
}

/// `--format`: the default (`file:line:col` with a code frame) or `tsc`
/// (`file(line,col)`, no frame, as tsc prints).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Pretty,
    Tsc,
}

/// Diagnostics as text for stderr.
#[must_use]
pub fn diagnostics(
    path: &str,
    source: &str,
    diags: &[Diag],
    format: Format,
    style: Style,
) -> String {
    let mut out = String::new();
    for diag in diags {
        let (line, col) = line_col(source, diag.start);
        let severity = match diag.severity {
            Severity::Error => style.red(diag.severity.as_str()),
            Severity::Warning => style.yellow(diag.severity.as_str()),
        };
        let code = if diag.code.is_empty() {
            String::new()
        } else {
            format!(" {}", style.bold(&diag.code))
        };
        match format {
            Format::Tsc => {
                let _ = writeln!(
                    out,
                    "{path}({line},{col}): {severity}{code}: {}",
                    diag.message
                );
                for (message, start, _) in &diag.related {
                    let (line, col) = line_col(source, *start);
                    let _ = writeln!(out, "  {path}({line},{col}): {message}");
                }
            }
            Format::Pretty => {
                let _ = writeln!(
                    out,
                    "{}: {severity}{code}: {}",
                    style.bold(&format!("{path}:{line}:{col}")),
                    style.bold(&diag.message)
                );
                out.push_str(&frame(source, diag.start, diag.end, style, diag.severity));
                for (message, start, end) in &diag.related {
                    let (line, col) = line_col(source, *start);
                    let _ = writeln!(
                        out,
                        "  {} {path}:{line}:{col}: {message}",
                        style.cyan("note:")
                    );
                    out.push_str(&frame(source, *start, *end, style, Severity::Warning));
                }
                if !diag.code.is_empty() && diag.code.starts_with("SWR") {
                    let _ = writeln!(
                        out,
                        "  {} tsc-ext explain {}",
                        style.dim("help:"),
                        diag.code
                    );
                }
            }
        }
    }
    out
}

/// The source line of `start` with a caret line under `start..end`.
fn frame(source: &str, start: u32, end: u32, style: Style, severity: Severity) -> String {
    let (line, col) = line_col(source, start);
    let Some(text) = source.split('\n').nth(line - 1) else {
        return String::new();
    };
    let text = text.trim_end_matches('\r');
    let (end_line, end_col) = line_col(source, end.max(start));
    let width = if end_line == line {
        end_col.saturating_sub(col).max(1)
    } else {
        text.chars().count().saturating_sub(col - 1).max(1)
    };
    let gutter = line.to_string();
    let pad = " ".repeat(gutter.len());
    let carets = "^".repeat(width);
    let carets = match severity {
        Severity::Error => style.red(&carets),
        Severity::Warning => style.yellow(&carets),
    };
    format!(
        "  {} {} {text}\n  {pad} {} {}{carets}\n",
        style.blue(&gutter),
        style.blue("|"),
        style.blue("|"),
        " ".repeat(col - 1)
    )
}

/// A unified diff from `before` to `after`, by lines, 3 lines of context.
#[must_use]
pub fn diff(path: &str, before: &str, after: &str, style: Style) -> String {
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();
    const CONTEXT: usize = 3;
    let ops = line_ops(&old, &new);
    let mut out = format!(
        "{}\n{}\n",
        style.bold(&format!("--- {path}")),
        style.bold(&format!("+++ {path} (expanded)"))
    );
    if ops.iter().all(|op| matches!(op, Op::Same(..))) {
        return out;
    }
    let mut index = 0;
    while index < ops.len() {
        if matches!(ops[index], Op::Same(..)) {
            index += 1;
            continue;
        }
        let start = index.saturating_sub(CONTEXT);
        let mut end = index;
        // Extend the hunk while changes are within 2 * CONTEXT of each other.
        let mut last_change = index;
        while end < ops.len() {
            if !matches!(ops[end], Op::Same(..)) {
                last_change = end;
            } else if end - last_change > 2 * CONTEXT {
                break;
            }
            end += 1;
        }
        let end = (last_change + CONTEXT + 1).min(ops.len());
        let (old_start, new_start) = positions(&ops, start);
        let old_len = ops[start..end]
            .iter()
            .filter(|op| !matches!(op, Op::Add(_)))
            .count();
        let new_len = ops[start..end]
            .iter()
            .filter(|op| !matches!(op, Op::Del(_)))
            .count();
        out.push_str(&style.cyan(&format!(
            "@@ -{},{old_len} +{},{new_len} @@",
            old_start + 1,
            new_start + 1
        )));
        out.push('\n');
        for op in &ops[start..end] {
            match op {
                Op::Same(i) => {
                    let _ = writeln!(out, " {}", old[*i]);
                }
                Op::Del(i) => {
                    let _ = writeln!(out, "{}", style.red(&format!("-{}", old[*i])));
                }
                Op::Add(j) => {
                    let _ = writeln!(out, "{}", style.green(&format!("+{}", new[*j])));
                }
            }
        }
        index = end;
    }
    out
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Same(usize),
    Del(usize),
    Add(usize),
}

fn positions(ops: &[Op], upto: usize) -> (usize, usize) {
    let mut old = 0;
    let mut new = 0;
    for op in &ops[..upto] {
        match op {
            Op::Same(..) => {
                old += 1;
                new += 1;
            }
            Op::Del(_) => old += 1,
            Op::Add(_) => new += 1,
        }
    }
    (old, new)
}

/// Longest-common-subsequence line diff. Quadratic; inputs are single
/// source files, and a very large pair falls back to delete-all/add-all.
fn line_ops(old: &[&str], new: &[&str]) -> Vec<Op> {
    if old.len().saturating_mul(new.len()) > 25_000_000 {
        let mut ops: Vec<Op> = (0..old.len()).map(Op::Del).collect();
        ops.extend((0..new.len()).map(Op::Add));
        return ops;
    }
    let (n, m) = (old.len(), new.len());
    let mut table = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i * (m + 1) + j] = if old[i] == new[j] {
                table[(i + 1) * (m + 1) + j + 1] + 1
            } else {
                table[(i + 1) * (m + 1) + j].max(table[i * (m + 1) + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut ops = Vec::new();
    while i < n && j < m {
        if old[i] == new[j] {
            ops.push(Op::Same(i));
            i += 1;
            j += 1;
        } else if table[(i + 1) * (m + 1) + j] >= table[i * (m + 1) + j + 1] {
            ops.push(Op::Del(i));
            i += 1;
        } else {
            ops.push(Op::Add(j));
            j += 1;
        }
    }
    ops.extend((i..n).map(Op::Del));
    ops.extend((j..m).map(Op::Add));
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_counts_characters() {
        assert_eq!(line_col("ab\ncé d", 7), (2, 4));
        assert_eq!(line_col("x", 99), (1, 2));
    }

    #[test]
    fn diff_marks_changed_lines() {
        let out = diff("a.sts", "a\nb\nc\n", "a\nB\nc\n", Style { on: false });
        assert!(out.contains("@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n"), "{out}");
    }

    #[test]
    fn frame_points_at_the_span() {
        let diag = Diag::new(Severity::Error, "SWR4001: No rule.", 10, 11, vec![]);
        assert_eq!(diag.code, "SWR4001");
        let out = diagnostics(
            "a.sts",
            "const a = x;\n",
            &[diag],
            Format::Pretty,
            Style { on: false },
        );
        assert!(
            out.contains(
                "a.sts:1:11: error SWR4001: No rule.\n  1 | const a = x;\n    |           ^\n"
            ),
            "{out}"
        );
    }
}
