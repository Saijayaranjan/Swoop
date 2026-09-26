//! Release tags and version ordering.

use semver::{BuildMetadata, Version};

/// Parse a release tag (`v1.2.3`, `1.2.3`, `v1.3.0-beta.1`) into a version. Build metadata is
/// dropped so it never decides ordering.
pub fn parse_tag(tag: &str) -> Option<Version> {
    let t = tag.trim();
    let t = t
        .strip_prefix('v')
        .or_else(|| t.strip_prefix('V'))
        .unwrap_or(t);
    let mut v = Version::parse(t).ok()?;
    v.build = BuildMetadata::EMPTY;
    Some(v)
}

/// Parse the running app's version the same way as a tag.
pub fn parse_version(s: &str) -> Option<Version> {
    parse_tag(s)
}

/// Is `candidate` strictly newer than `current` (semver precedence, build metadata ignored)?
pub fn is_newer(candidate: &Version, current: &Version) -> bool {
    let strip = |v: &Version| {
        let mut v = v.clone();
        v.build = BuildMetadata::EMPTY;
        v
    };
    strip(candidate) > strip(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        parse_tag(s).unwrap()
    }

    #[test]
    fn tags_parse() {
        assert_eq!(v("v1.2.3"), Version::new(1, 2, 3));
        assert_eq!(v("1.2.3"), Version::new(1, 2, 3));
        assert_eq!(v(" V0.1.1 "), Version::new(0, 1, 1));
        assert_eq!(v("v1.3.0-beta.1").pre.as_str(), "beta.1");
        assert_eq!(v("v1.0.0+abc"), Version::new(1, 0, 0));
        for bad in ["", "v", "v1.2", "release-1.0.0", "latest", "v1.2.3.4"] {
            assert!(parse_tag(bad).is_none(), "{bad} should not parse");
        }
    }

    #[test]
    fn ordering() {
        assert!(is_newer(&v("0.1.1"), &v("0.1.0")));
        assert!(is_newer(&v("0.2.0"), &v("0.1.9")));
        assert!(is_newer(&v("1.0.0"), &v("0.99.99")));
        assert!(is_newer(&v("0.10.0"), &v("0.9.0")), "numeric, not lexical");
        assert!(!is_newer(&v("0.1.0"), &v("0.1.0")));
        assert!(!is_newer(&v("0.0.9"), &v("0.1.0")));
        // pre-releases sort before their release, after the previous one
        assert!(is_newer(&v("1.0.0"), &v("1.0.0-beta.2")));
        assert!(is_newer(&v("1.0.0-beta.2"), &v("1.0.0-beta.1")));
        assert!(is_newer(&v("1.0.0-rc.1"), &v("0.9.0")));
        assert!(!is_newer(&v("1.0.0-beta.1"), &v("1.0.0")));
        // build metadata never makes a version "newer"
        let a = Version::parse("1.0.0+2").unwrap();
        let b = Version::parse("1.0.0+1").unwrap();
        assert!(!is_newer(&a, &b));
    }
}
