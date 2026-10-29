//! Versions compared the semver way. Here rather than in Settings because three programs
//! ask "is this release newer than me?" about the same `update.json`: Settings (its check
//! and About), the hub (whether to offer it) and the island (its tray entry).

use std::cmp::Ordering;

/// `major.minor.patch[-pre][+build]`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Version {
    core: [u64; 3],
    pre: Vec<String>,
}

impl Version {
    fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_start_matches(['v', 'V']);
        let text = text.split('+').next()?;
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, pre.split('.').map(String::from).collect()),
            None => (text, Vec::new()),
        };
        let mut numbers = core.split('.').map(|n| n.parse::<u64>().ok());
        let core = [numbers.next()??, numbers.next().flatten().unwrap_or(0), numbers.next().flatten().unwrap_or(0)];
        Some(Self { core, pre })
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core.cmp(&other.core).then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            // A pre-release comes before its release.
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                for (a, b) in self.pre.iter().zip(&other.pre) {
                    let order = match (a.parse::<u64>(), b.parse::<u64>()) {
                        (Ok(x), Ok(y)) => x.cmp(&y),
                        (Ok(_), Err(_)) => Ordering::Less,
                        (Err(_), Ok(_)) => Ordering::Greater,
                        (Err(_), Err(_)) => a.cmp(b),
                    };
                    if order != Ordering::Equal {
                        return order;
                    }
                }
                self.pre.len().cmp(&other.pre.len())
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Whether `text` reads as a version at all.
pub fn valid(text: &str) -> bool {
    Version::parse(text).is_some()
}

/// Whether `candidate` is a later version than `current` (unreadable versions never are).
pub fn newer(candidate: &str, current: &str) -> bool {
    match (Version::parse(candidate), Version::parse(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_the_semver_way() {
        assert!(newer("0.2.0", "0.1.0"));
        assert!(newer("v0.10.0", "0.9.9"));
        assert!(newer("1.0", "0.99.99"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.2.0"));
        assert!(newer("0.2.0", "0.2.0-beta.2"), "a release beats its pre-release");
        assert!(!newer("0.2.0-beta.2", "0.2.0"));
        assert!(newer("0.2.0-beta.10", "0.2.0-beta.2"), "numbers compare as numbers");
        assert!(newer("0.2.0-rc.1", "0.2.0-beta.9"));
        assert!(newer("0.2.0-beta.1.1", "0.2.0-beta.1"));
        assert!(!newer("0.2.0+build.5", "0.2.0"), "build metadata doesn't count");
        assert!(!newer("latest", "0.1.0"));
        assert!(!newer("0.2.0", "nonsense"));
        assert!(valid("v1.2.3") && !valid("nightly"));
    }
}
