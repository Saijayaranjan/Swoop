//! Parse `MLSD` facts and classic `LIST` (UNIX `ls -l`) lines.

use crate::client::FtpEntry;

pub fn parse_mlsd_line(line: &str) -> Option<FtpEntry> {
    let (facts, name) = line.split_once(' ')?;
    let mut entry = FtpEntry {
        name: name.trim().to_owned(),
        size: None,
        is_dir: false,
        modified: None,
    };
    if entry.name.is_empty() || entry.name == "." || entry.name == ".." {
        return None;
    }
    for fact in facts.split(';') {
        let Some((k, v)) = fact.split_once('=') else {
            continue;
        };
        match k.to_ascii_lowercase().as_str() {
            "type" => {
                let t = v.to_ascii_lowercase();
                if t == "cdir" || t == "pdir" {
                    return None;
                }
                entry.is_dir = t == "dir";
            }
            "size" => entry.size = v.parse().ok(),
            "modify" => entry.modified = Some(v.to_owned()),
            _ => {}
        }
    }
    Some(entry)
}

/// `-rw-r--r--   1 user group   12345 Jan  1 12:00 name with spaces`
pub fn parse_list_line(line: &str) -> Option<FtpEntry> {
    let cols: Vec<&str> = line.split_whitespace().collect();
    if cols.len() < 9 {
        // DOS-style: `01-01-25  12:00PM       12345 name` or `<DIR>`
        if cols.len() >= 4 {
            let is_dir = cols[2].eq_ignore_ascii_case("<dir>");
            let size = if is_dir { None } else { cols[2].parse().ok() };
            let name = cols[3..].join(" ");
            return Some(FtpEntry {
                name,
                size,
                is_dir,
                modified: Some(format!("{} {}", cols[0], cols[1])),
            });
        }
        return None;
    }
    let perms = cols[0];
    let is_dir = perms.starts_with('d');
    let is_link = perms.starts_with('l');
    let size = cols[4].parse().ok();
    let modified = Some(format!("{} {} {}", cols[5], cols[6], cols[7]));
    let mut name = cols[8..].join(" ");
    if is_link {
        if let Some((n, _)) = name.split_once(" -> ") {
            name = n.to_owned();
        }
    }
    if name == "." || name == ".." {
        return None;
    }
    Some(FtpEntry {
        name,
        size: if is_dir { None } else { size },
        is_dir,
        modified,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mlsd() {
        let e =
            parse_mlsd_line("type=file;size=1234;modify=20250101120000; report final.pdf").unwrap();
        assert_eq!(e.name, "report final.pdf");
        assert_eq!(e.size, Some(1234));
        assert!(!e.is_dir);
        assert!(parse_mlsd_line("type=cdir; .").is_none());
        assert!(parse_mlsd_line("type=dir; sub").unwrap().is_dir);
    }

    #[test]
    fn parses_list() {
        let e = parse_list_line("-rw-r--r--   1 ftp  ftp      2048 Mar 10 09:15 a b.zip").unwrap();
        assert_eq!(e.name, "a b.zip");
        assert_eq!(e.size, Some(2048));
        let d = parse_list_line("drwxr-xr-x   2 ftp  ftp      4096 Mar 10 09:15 dir").unwrap();
        assert!(d.is_dir);
        let w = parse_list_line("01-01-25  12:00PM       12345 file.txt").unwrap();
        assert_eq!(w.size, Some(12345));
        let dd = parse_list_line("01-01-25  12:00PM       <DIR>  folder").unwrap();
        assert!(dd.is_dir);
    }
}
