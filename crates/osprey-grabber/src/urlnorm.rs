//! URL normalisation and scope rules.

use url::Url;

/// Resolve `href` against `base`, drop fragments, lower-case the host, strip default ports and
/// common tracking parameters. Returns `None` for non-http(s) or unparsable links.
pub fn normalize(base: &Url, href: &str) -> Option<Url> {
    let href = href.trim();
    if href.is_empty()
        || href.starts_with('#')
        || href.starts_with("javascript:")
        || href.starts_with("mailto:")
        || href.starts_with("tel:")
        || href.starts_with("data:")
    {
        return None;
    }
    let mut u = base.join(href).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    u.set_fragment(None);
    if let Some(h) = u.host_str().map(|h| h.to_ascii_lowercase()) {
        let _ = u.set_host(Some(&h));
    }
    if u.port() == Some(80) && u.scheme() == "http"
        || u.port() == Some(443) && u.scheme() == "https"
    {
        let _ = u.set_port(None);
    }
    let tracking = [
        "utm_source",
        "utm_medium",
        "utm_campaign",
        "utm_term",
        "utm_content",
        "fbclid",
        "gclid",
        "mc_cid",
        "mc_eid",
        "ref_src",
    ];
    if u.query().is_some() {
        let kept: Vec<(String, String)> = u
            .query_pairs()
            .filter(|(k, _)| !tracking.contains(&k.as_ref()))
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if kept.is_empty() {
            u.set_query(None);
        } else {
            let mut qs = url::form_urlencoded::Serializer::new(String::new());
            for (k, v) in kept {
                qs.append_pair(&k, &v);
            }
            u.set_query(Some(&qs.finish()));
        }
    }
    Some(u)
}

/// Registrable-ish domain: the last two labels (good enough for scope checks without a PSL).
pub fn base_domain(host: &str) -> String {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() <= 2 {
        return host.to_ascii_lowercase();
    }
    parts[parts.len() - 2..].join(".").to_ascii_lowercase()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    SameDomain,
    Subdomains,
    External,
}

impl Scope {
    pub fn parse(s: &str) -> Scope {
        match s {
            "subdomains" => Scope::Subdomains,
            "external" => Scope::External,
            _ => Scope::SameDomain,
        }
    }

    pub fn allows(self, start: &Url, candidate: &Url) -> bool {
        let (Some(a), Some(b)) = (start.host_str(), candidate.host_str()) else {
            return false;
        };
        match self {
            Scope::External => true,
            Scope::SameDomain => a.eq_ignore_ascii_case(b),
            Scope::Subdomains => {
                let bd = base_domain(a);
                b.eq_ignore_ascii_case(a)
                    || b.to_ascii_lowercase().ends_with(&format!(".{bd}"))
                    || b.eq_ignore_ascii_case(&bd)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises() {
        let base = Url::parse("https://Example.com:443/dir/page.html").unwrap();
        assert_eq!(
            normalize(&base, "../a.pdf#top").unwrap().as_str(),
            "https://example.com/a.pdf"
        );
        assert_eq!(
            normalize(&base, "x?utm_source=1&id=2").unwrap().as_str(),
            "https://example.com/dir/x?id=2"
        );
        assert!(normalize(&base, "javascript:void(0)").is_none());
        assert!(normalize(&base, "ftp://x/y").is_none());
    }

    #[test]
    fn scopes() {
        let start = Url::parse("https://www.example.com/").unwrap();
        let sub = Url::parse("https://cdn.example.com/f").unwrap();
        let other = Url::parse("https://other.org/f").unwrap();
        assert!(Scope::SameDomain.allows(&start, &start));
        assert!(!Scope::SameDomain.allows(&start, &sub));
        assert!(Scope::Subdomains.allows(&start, &sub));
        assert!(!Scope::Subdomains.allows(&start, &other));
        assert!(Scope::External.allows(&start, &other));
    }
}
