//! A small robots.txt evaluator: longest-match Allow/Disallow for the matching user-agent
//! group (ours, else `*`), plus `Crawl-delay`.

/// (agents, rules, crawl-delay) for one `User-agent` group.
type Group = (Vec<String>, Vec<(bool, String)>, Option<u64>);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Robots {
    rules: Vec<(bool, String)>, // (allow, path prefix)
    pub crawl_delay_ms: Option<u64>,
}

impl Robots {
    /// Everything allowed (missing or unparsable robots.txt).
    pub fn permissive() -> Self {
        Self::default()
    }

    pub fn parse(text: &str, our_agent: &str) -> Self {
        let our = our_agent.to_ascii_lowercase();
        let mut groups: Vec<Group> = Vec::new();
        let mut current: Option<Group> = None;
        let mut last_was_agent = false;
        for raw in text.lines() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((k, v)) = line.split_once(':') else {
                continue;
            };
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim();
            match key.as_str() {
                "user-agent" => {
                    if last_was_agent {
                        if let Some(c) = &mut current {
                            c.0.push(val.to_ascii_lowercase());
                        }
                    } else {
                        if let Some(c) = current.take() {
                            groups.push(c);
                        }
                        current = Some((vec![val.to_ascii_lowercase()], Vec::new(), None));
                    }
                    last_was_agent = true;
                }
                "disallow" | "allow" => {
                    last_was_agent = false;
                    if let Some(c) = &mut current {
                        if !val.is_empty() {
                            c.1.push((key == "allow", val.to_owned()));
                        }
                    }
                }
                "crawl-delay" => {
                    last_was_agent = false;
                    if let Some(c) = &mut current {
                        c.2 = val.parse::<f64>().ok().map(|s| (s * 1000.0) as u64);
                    }
                }
                _ => last_was_agent = false,
            }
        }
        if let Some(c) = current.take() {
            groups.push(c);
        }
        let pick = groups
            .iter()
            .find(|g| {
                g.0.iter()
                    .any(|a| !a.is_empty() && a != "*" && our.contains(a.as_str()))
            })
            .or_else(|| groups.iter().find(|g| g.0.iter().any(|a| a == "*")));
        match pick {
            Some(g) => Self {
                rules: g.1.clone(),
                crawl_delay_ms: g.2,
            },
            None => Self::permissive(),
        }
    }

    /// Longest matching rule wins; Allow wins ties. `$` anchors and `*` wildcards are supported.
    pub fn allows(&self, path: &str) -> bool {
        let mut best: Option<(usize, bool)> = None;
        for (allow, pattern) in &self.rules {
            if pattern_matches(pattern, path) {
                let len = pattern.len();
                match best {
                    Some((l, a)) if l > len || (l == len && a) => {}
                    _ => best = Some((len, *allow)),
                }
            }
        }
        best.map(|(_, a)| a).unwrap_or(true)
    }
}

fn pattern_matches(pattern: &str, path: &str) -> bool {
    let anchored = pattern.ends_with('$');
    let pat = pattern.trim_end_matches('$');
    if !pat.contains('*') {
        return if anchored {
            path == pat
        } else {
            path.starts_with(pat)
        };
    }
    // wildcard prefix match
    let parts: Vec<&str> = pat.split('*').collect();
    let mut pos = 0usize;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            if !path.starts_with(part) {
                return false;
            }
            pos = part.len();
        } else if let Some(found) = path[pos..].find(part) {
            pos += found + part.len();
        } else {
            return false;
        }
    }
    !anchored || pos == path.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_rules() {
        let r = Robots::parse("User-agent: *\nDisallow: /private/\nAllow: /private/public\nDisallow: /*.pdf$\nCrawl-delay: 2\n\nUser-agent: Swoop\nDisallow: /swoop-only\n", "Swoop/0.1");
        // our specific group is chosen
        assert!(!r.allows("/swoop-only/x"));
        assert!(r.allows("/private/x"));
        let g = Robots::parse("User-agent: *\nDisallow: /private/\nAllow: /private/public\nDisallow: /*.pdf$\nCrawl-delay: 2\n", "Swoop/0.1");
        assert!(!g.allows("/private/x"));
        assert!(g.allows("/private/public/y"));
        assert!(!g.allows("/docs/a.pdf"));
        assert!(g.allows("/docs/a.pdf?x"));
        assert!(g.allows("/"));
        assert_eq!(g.crawl_delay_ms, Some(2000));
        assert!(Robots::parse("", "x").allows("/anything"));
    }
}
