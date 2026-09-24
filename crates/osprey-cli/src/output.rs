//! Human-readable and JSON output formatting.

use humansize::{format_size, BINARY};
use serde::Serialize;

/// Print any serializable value as pretty JSON (used for `--json`).
pub fn print_json(value: &impl Serialize) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("failed to serialize output: {e}"),
    }
}

/// `1.2 MiB`.
pub fn size(bytes: u64) -> String {
    format_size(bytes, BINARY)
}

/// `1.2 MiB/s`.
pub fn speed(bytes_per_sec: u64) -> String {
    format!("{}/s", size(bytes_per_sec))
}

/// `-` when unknown, otherwise `1.2 MiB`.
pub fn size_opt(bytes: Option<u64>) -> String {
    match bytes {
        Some(b) => size(b),
        None => "-".to_owned(),
    }
}

/// `12.3%` or `-` when the total is unknown.
pub fn percent(p: Option<f32>) -> String {
    match p {
        Some(p) => format!("{p:.1}%"),
        None => "-".to_owned(),
    }
}

/// `1h02m03s`, `2m03s`, `45s`, or `-` when unknown.
pub fn eta(seconds: Option<u64>) -> String {
    match seconds {
        None => "-".to_owned(),
        Some(s) => humanize_duration(s),
    }
}

pub fn humanize_duration(total_seconds: u64) -> String {
    let h = total_seconds / 3600;
    let m = (total_seconds % 3600) / 60;
    let s = total_seconds % 60;
    if h > 0 {
        format!("{h}h{m:02}m{s:02}s")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

/// A simple left-aligned, column-width-adjusted text table for terminal output.
pub struct Table {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(headers: &[&str]) -> Self {
        Self {
            headers: headers.iter().map(|s| s.to_string()).collect(),
            rows: Vec::new(),
        }
    }

    pub fn push(&mut self, row: Vec<String>) {
        self.rows.push(row);
    }

    pub fn print(&self) {
        if self.rows.is_empty() {
            println!("(none)");
            return;
        }
        let cols = self.headers.len();
        let mut widths: Vec<usize> = self.headers.iter().map(|h| h.chars().count()).collect();
        for row in &self.rows {
            for (width, cell) in widths.iter_mut().zip(row.iter()) {
                *width = (*width).max(cell.chars().count());
            }
        }
        let print_row = |cells: &[String]| {
            let mut line = String::new();
            for (i, width) in widths.iter().enumerate().take(cols) {
                let cell = cells.get(i).map(String::as_str).unwrap_or("");
                if i + 1 == cols {
                    line.push_str(cell);
                } else {
                    line.push_str(&format!("{cell:<width$}  "));
                }
            }
            println!("{line}");
        };
        print_row(&self.headers);
        self.rows.iter().for_each(|r| print_row(r));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_formatting() {
        assert_eq!(humanize_duration(5), "5s");
        assert_eq!(humanize_duration(65), "1m05s");
        assert_eq!(humanize_duration(3725), "1h02m05s");
    }

    #[test]
    fn eta_none_is_dash() {
        assert_eq!(eta(None), "-");
        assert_eq!(eta(Some(0)), "0s");
    }

    #[test]
    fn size_formatting_is_binary() {
        assert_eq!(size(1024), "1 KiB");
        assert_eq!(size_opt(None), "-");
        assert_eq!(size_opt(Some(0)), "0 B");
    }

    #[test]
    fn percent_formatting() {
        assert_eq!(percent(Some(12.34)), "12.3%");
        assert_eq!(percent(None), "-");
    }
}
