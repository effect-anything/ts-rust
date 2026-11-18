//! `cheatsheet` and `explain`: the docs embedded in the binary, and a light
//! Markdown renderer for the terminal.

use std::fmt::Write as _;
use ts_ext_sweetener::catalog::{CATALOG, Entry, lookup};

use super::render::Style;

/// `ext/docs/cheatsheet.md`: one section (`## Topic`) per topic.
pub const CHEATSHEET: &str = include_str!("../../../../ext/docs/cheatsheet.md");
/// `ext/docs/options.md`.
pub const OPTIONS: &str = include_str!("../../../../ext/docs/options.md");
/// `ext/docs/diagnostics.md`, generated from the catalog.
pub const DIAGNOSTICS: &str = include_str!("../../../../ext/docs/diagnostics.md");

/// The topic names: each `## ` heading of the cheatsheet, lowercased to its
/// first word, then `options` and `diagnostics`.
#[must_use]
pub fn topics() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = CHEATSHEET
        .lines()
        .filter_map(|line| line.strip_prefix("## "))
        .map(|heading| (slug(heading), heading.to_owned()))
        .collect();
    out.push((
        "options".to_owned(),
        "tsconfig options reference".to_owned(),
    ));
    out.push((
        "diagnostics".to_owned(),
        "every SWR code (see also `tsc-ext explain`)".to_owned(),
    ));
    out
}

fn slug(heading: &str) -> String {
    heading
        .split(|c: char| !c.is_ascii_alphanumeric())
        .find(|word| !word.is_empty())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// The Markdown of `topic` (`None`: the whole cheatsheet).
#[must_use]
pub fn topic(name: Option<&str>) -> Option<String> {
    let Some(name) = name else {
        return Some(CHEATSHEET.to_owned());
    };
    let name = name.to_ascii_lowercase();
    match name.as_str() {
        "options" => return Some(OPTIONS.to_owned()),
        "diagnostics" | "codes" => return Some(DIAGNOSTICS.to_owned()),
        _ => {}
    }
    let mut out = String::new();
    let mut inside = false;
    for line in CHEATSHEET.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            inside = slug(heading) == name;
        } else if line.starts_with("# ") {
            inside = false;
        }
        if inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    (!out.is_empty()).then_some(out)
}

/// One catalog entry as Markdown.
#[must_use]
pub fn entry_markdown(entry: &Entry) -> String {
    let mut out = format!(
        "# {} ({})\n\n**{}**\n\n{}\n",
        entry.code,
        entry.level.as_str(),
        entry.title,
        entry.explanation
    );
    if !entry.bad.is_empty() {
        for (name, text) in entry.files {
            let _ = write!(out, "\n`{name}`:\n\n```ts\n{text}```\n");
        }
        let _ = write!(out, "\nTriggers it:\n\n```ts\n{}```\n", entry.bad);
    }
    let _ = write!(out, "\nFix:\n\n{}\n", entry.fix);
    out
}

/// `explain` for a code: the entry, or the codes that start with `code`.
///
/// # Errors
/// An unknown code, with the codes near it.
pub fn explain(code: &str) -> Result<String, String> {
    if let Some(entry) = lookup(code) {
        return Ok(entry_markdown(entry));
    }
    let wanted = code.trim().to_ascii_uppercase();
    let wanted = wanted.trim_start_matches("SWR");
    let near: Vec<String> = CATALOG
        .iter()
        .filter(|entry| {
            !wanted.is_empty() && entry.code[3..].starts_with(&wanted[..1.min(wanted.len())])
        })
        .map(|entry| format!("  {}  {}", entry.code, entry.title))
        .collect();
    Err(if near.is_empty() {
        format!("unknown diagnostic code {code}; `tsc-ext explain --list` lists them")
    } else {
        format!(
            "unknown diagnostic code {code}. Codes in that range:\n{}",
            near.join("\n")
        )
    })
}

/// Every code with its title, one per line.
#[must_use]
pub fn list() -> String {
    CATALOG.iter().fold(String::new(), |mut out, entry| {
        let _ = writeln!(
            out,
            "{}  {:<7}  {}",
            entry.code,
            entry.level.as_str(),
            entry.title
        );
        out
    })
}

/// Markdown for a terminal: headings bold, code blocks indented, inline
/// code colored, `**bold**` and link targets removed.
#[must_use]
pub fn render(markdown: &str, style: Style) -> String {
    let mut out = String::new();
    let mut fence = false;
    let mut table: Vec<Vec<String>> = Vec::new();
    for line in markdown.lines() {
        if !line.starts_with('|') && !table.is_empty() {
            flush_table(&mut out, &mut table);
        }
        if line.trim_start().starts_with("```") {
            fence = !fence;
            continue;
        }
        if fence {
            let _ = writeln!(out, "    {}", style.cyan(line));
            continue;
        }
        if line.starts_with("<!--") {
            continue;
        }
        if let Some(heading) = line.strip_prefix("# ") {
            let text = inline(heading, style);
            let _ = writeln!(out, "{}", style.bold(&text));
            if !style.on {
                let _ = writeln!(out, "{}", "=".repeat(heading.chars().count()));
            }
            continue;
        }
        if let Some(heading) = line
            .strip_prefix("## ")
            .or_else(|| line.strip_prefix("### "))
        {
            let text = inline(heading, style);
            let _ = writeln!(out, "{}", style.blue(&text));
            if !style.on {
                let _ = writeln!(out, "{}", "-".repeat(heading.chars().count()));
            }
            continue;
        }
        if let Some(item) = line.strip_prefix("- ") {
            let _ = writeln!(out, "  • {}", inline(item, style));
            continue;
        }
        if line.starts_with('|') {
            // Tables: drop the |---| rule, keep the rows.
            if line.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')) {
                continue;
            }
            table.push(
                table_cells(line)
                    .iter()
                    .map(|cell| inline(cell, style))
                    .collect(),
            );
            continue;
        }
        out.push_str(&inline(line, style));
        out.push('\n');
    }
    flush_table(&mut out, &mut table);
    out
}

/// The cells of a table row; `\|` is a `|` inside a cell.
fn table_cells(line: &str) -> Vec<String> {
    let inner = line.trim().trim_start_matches('|');
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    let mut cells = vec![String::new()];
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                chars.next();
                cells.last_mut().unwrap().push('|');
            }
            '|' => cells.push(String::new()),
            c => cells.last_mut().unwrap().push(c),
        }
    }
    cells.iter().map(|cell| cell.trim().to_owned()).collect()
}

/// Width on screen: ANSI color sequences take none.
fn visible_width(text: &str) -> usize {
    let mut width = 0;
    let mut escape = false;
    for c in text.chars() {
        match (escape, c) {
            (false, '\u{1b}') => escape = true,
            (true, 'm') => escape = false,
            (true, _) => {}
            (false, _) => width += 1,
        }
    }
    width
}

/// Print the buffered rows with aligned columns (the last one unpadded).
fn flush_table(out: &mut String, table: &mut Vec<Vec<String>>) {
    let columns = table.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|column| {
            table
                .iter()
                .filter_map(|row| row.get(column))
                .map(|cell| visible_width(cell))
                .max()
                .unwrap_or(0)
        })
        .collect();
    for row in table.drain(..) {
        let mut line = String::from("  ");
        for (column, cell) in row.iter().enumerate() {
            line.push_str(cell);
            if column + 1 < row.len() {
                line.push_str(&" ".repeat(widths[column] - visible_width(cell)));
                line.push_str("  │  ");
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
}

fn inline(text: &str, style: Style) -> String {
    let mut out = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("**")
            && let Some(end) = after.find("**")
        {
            out.push_str(&style.bold(&after[..end]));
            rest = &after[end + 2..];
        } else if let Some(after) = rest.strip_prefix('`')
            && let Some(end) = after.find('`')
        {
            if style.on {
                out.push_str(&style.cyan(&after[..end]));
            } else {
                out.push('`');
                out.push_str(&after[..end]);
                out.push('`');
            }
            rest = &after[end + 1..];
        } else if let Some(after) = rest.strip_prefix('[')
            && let Some(close) = after.find("](")
            && let Some(end) = after[close..].find(')')
        {
            out.push_str(&after[..close]);
            rest = &after[close + end + 1..];
        } else {
            let ch = rest.chars().next().unwrap_or(' ');
            out.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_topic_renders() {
        for (name, _) in topics() {
            assert!(
                topic(Some(&name)).is_some_and(|text| !text.is_empty()),
                "{name}"
            );
        }
        assert!(topic(Some("nope")).is_none());
    }

    #[test]
    fn plain_rendering_keeps_code_readable() {
        let out = render(
            "# T\n\nUse `x` and **y** [z](#z).\n```ts\nconst a = 1;\n```\n",
            Style { on: false },
        );
        assert_eq!(out, "T\n=\n\nUse `x` and y z.\n    const a = 1;\n");
    }

    #[test]
    fn explain_finds_codes_and_suggests() {
        assert!(
            explain("SWR4001")
                .unwrap()
                .contains("No rule of the macro accepted")
        );
        assert!(explain("4001").is_ok());
        assert!(explain("SWR4999").unwrap_err().contains("SWR4001"));
    }
}
