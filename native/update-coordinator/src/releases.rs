//! Bounded, read-only discovery of official macOS release assets.
use crate::{origin::command_output, InstallOrigin};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
use std::path::Path;
use std::time::{Duration, Instant};

const REPOSITORY: &str = "hanbong5938/herdr-desktop-pet";
const API: &str = "https://api.github.com/repos/hanbong5938/herdr-desktop-pet/releases";
const DOWNLOAD: &str = "https://github.com/hanbong5938/herdr-desktop-pet/releases/download/";
const PAGE_SIZE: usize = 50;
const MAX_PAGES: usize = 4;
const MAX_PAGE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePolicy {
    #[default]
    StableOnly,
    StableAndBeta,
}

impl UpdatePolicy {
    pub fn allows_version(self, version: &str) -> Result<bool, String> {
        let version =
            Version::parse(version).map_err(|e| format!("invalid release version: {e}"))?;
        if version.build.is_empty() && version.pre.is_empty() {
            return Ok(true);
        }
        Ok(self == Self::StableAndBeta && official_beta(&version))
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseChannel {
    Stable,
    Beta,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReleaseCandidate {
    pub version: String,
    pub channel: ReleaseChannel,
    pub tag: String,
    pub release_id: u64,
    pub asset_name: String,
    pub archive_url: String,
    pub archive_size: u64,
    pub sha256: String,
    pub checksum_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReleaseTarget {
    pub policy: UpdatePolicy,
    pub release: ReleaseCandidate,
    pub origin: InstallOrigin,
    pub resolved_commit: Option<String>,
}

fn official_beta(version: &Version) -> bool {
    if !version.build.is_empty() {
        return false;
    }
    let pre = version.pre.as_str();
    let Some(n) = pre.strip_prefix("beta.") else {
        return false;
    };
    !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && (n == "0" || !n.starts_with('0'))
}

fn classify(version: &str) -> Result<(Version, ReleaseChannel, String), String> {
    let parsed = Version::parse(version).map_err(|e| format!("invalid release version: {e}"))?;
    if parsed.to_string() != version || !parsed.build.is_empty() {
        return Err("release version is not canonical".into());
    }
    let (channel, tag) = if parsed.pre.is_empty() {
        (ReleaseChannel::Stable, format!("v{version}"))
    } else if official_beta(&parsed) {
        (ReleaseChannel::Beta, format!("beta/{version}"))
    } else {
        return Err("not an official stable or beta version".into());
    };
    Ok((parsed, channel, tag))
}

fn download_url(tag: &str, asset: &str) -> String {
    format!("{DOWNLOAD}{tag}/{asset}")
}

fn trusted_url(url: &str, tag: &str, asset: &str) -> bool {
    url == download_url(tag, asset)
        || (tag.starts_with("beta/") && url == download_url(&tag.replacen('/', "%2F", 1), asset))
}

impl ReleaseCandidate {
    /// Check every portable release identity field, including the exact official download paths.
    pub fn validate(&self) -> Result<(), String> {
        let (_, channel, tag) = classify(&self.version)?;
        if self.channel != channel || self.tag != tag || self.release_id == 0 {
            return Err("release channel, tag or id does not match version".into());
        }
        let asset = format!("HerdrDesktopPet-v{}-macos-arm64.tar.gz", self.version);
        if self.asset_name != asset || self.archive_size == 0 {
            return Err("release archive name or size is invalid".into());
        }
        if !trusted_url(&self.archive_url, &tag, &asset)
            || !trusted_url(&self.checksum_url, &tag, &format!("{asset}.sha256"))
        {
            return Err("release asset URL is not the official repository/tag/name".into());
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err("release archive SHA-256 is invalid".into());
        }
        Ok(())
    }

    /// Verify the downloaded per-archive checksum before using the catalog digest.
    pub fn validate_checksum(&self, checksum: &str) -> Result<(), String> {
        self.validate()?;
        if checksum.len() > 4096 {
            return Err("release checksum exceeds bound".into());
        }
        let mut matches = 0;
        for line in checksum.lines() {
            let Some((digest, name)) = line.split_once(' ') else {
                continue;
            };
            if name.trim_start_matches([' ', '*']) == self.asset_name {
                if !digest.eq_ignore_ascii_case(&self.sha256) {
                    return Err("release checksum conflicts with catalog digest".into());
                }
                matches += 1;
            }
        }
        if matches != 1 {
            return Err("checksum must contain exactly one matching archive digest".into());
        }
        Ok(())
    }
}

fn asset<'a>(assets: &'a [Value], name: &str, tag: &str) -> Option<&'a Value> {
    let mut matching = assets
        .iter()
        .filter(|a| a.get("name").and_then(Value::as_str) == Some(name));
    let result = matching.next()?;
    if matching.next().is_some()
        || result.get("state").and_then(Value::as_str) != Some("uploaded")
        || result
            .get("size")
            .and_then(Value::as_u64)
            .filter(|s| *s > 0)
            .is_none()
        || !result
            .get("browser_download_url")
            .and_then(Value::as_str)
            .is_some_and(|url| trusted_url(url, tag, name))
    {
        return None;
    }
    Some(result)
}

/// Parse one official GitHub releases API page; unqualified releases are ignored.
pub fn parse_catalog_page(json: &str) -> Result<Vec<ReleaseCandidate>, String> {
    if json.len() > MAX_PAGE_BYTES {
        return Err("release catalog page exceeds bound".into());
    }
    let page: Vec<Value> =
        serde_json::from_str(json).map_err(|e| format!("invalid release catalog: {e}"))?;
    if page.len() > PAGE_SIZE {
        return Err("release catalog page exceeds entry bound".into());
    }
    let mut candidates = Vec::new();
    for release in &page {
        if release.get("draft").and_then(Value::as_bool) != Some(false)
            || release
                .get("published_at")
                .and_then(Value::as_str)
                .is_none()
        {
            continue;
        }
        let Some(tag) = release.get("tag_name").and_then(Value::as_str) else {
            continue;
        };
        let version = tag.strip_prefix('v').or_else(|| tag.strip_prefix("beta/"));
        let Some(version) = version else { continue };
        let Ok((_, channel, expected_tag)) = classify(version) else {
            continue;
        };
        let Some(id) = release
            .get("id")
            .and_then(Value::as_u64)
            .filter(|id| *id > 0)
        else {
            continue;
        };
        if tag != expected_tag
            || release.get("prerelease").and_then(Value::as_bool)
                != Some(channel == ReleaseChannel::Beta)
            || release.get("url").and_then(Value::as_str) != Some(format!("{API}/{id}").as_str())
            || !release
                .get("html_url")
                .and_then(Value::as_str)
                .is_some_and(|url| {
                    url == format!("https://github.com/{REPOSITORY}/releases/tag/{tag}")
                        || (channel == ReleaseChannel::Beta
                            && url
                                == format!(
                                    "https://github.com/{REPOSITORY}/releases/tag/{}",
                                    tag.replace('/', "%2F")
                                ))
                })
        {
            continue;
        }
        let Some(assets) = release
            .get("assets")
            .and_then(Value::as_array)
            .filter(|a| a.len() <= 100)
        else {
            continue;
        };
        let name = format!("HerdrDesktopPet-v{version}-macos-arm64.tar.gz");
        let Some(archive) = asset(assets, &name, tag) else {
            continue;
        };
        let Some(checksum) = asset(assets, &format!("{name}.sha256"), tag) else {
            continue;
        };
        if asset(assets, "SHA256SUMS", tag).is_none() {
            continue;
        }
        let Some(sha256) = archive
            .get("digest")
            .and_then(Value::as_str)
            .and_then(|d| d.strip_prefix("sha256:"))
        else {
            continue;
        };
        let candidate = ReleaseCandidate {
            version: version.into(),
            channel,
            tag: tag.into(),
            release_id: id,
            asset_name: name,
            archive_url: archive["browser_download_url"]
                .as_str()
                .unwrap_or_default()
                .into(),
            archive_size: archive["size"].as_u64().unwrap_or_default(),
            sha256: sha256.into(),
            checksum_url: checksum["browser_download_url"]
                .as_str()
                .unwrap_or_default()
                .into(),
        };
        if candidate.validate().is_ok() {
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

/// Perform an anonymous read-only catalog request. Call this from a background check worker,
/// never from the AppKit main thread. Four pages and one overall deadline bound the operation.
pub fn fetch_candidates() -> Result<Vec<ReleaseCandidate>, String> {
    let deadline = Instant::now() + Duration::from_secs(40);
    let mut result = Vec::new();
    for page in 1..=MAX_PAGES {
        // Reserve a second for process shutdown inside command_output's own deadline.
        let remaining = deadline.saturating_duration_since(Instant::now()).as_secs();
        let seconds = remaining.saturating_sub(1).min(10);
        if seconds == 0 {
            return Err("release catalog deadline exceeded".into());
        }
        let url = format!("{API}?per_page={PAGE_SIZE}&page={page}");
        let text = command_output(
            Path::new("/usr/bin/curl"),
            &[
                "--disable",
                "--fail",
                "--silent",
                "--show-error",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--connect-timeout",
                "5",
                "--max-time",
                &seconds.to_string(),
                "--header",
                "Accept: application/vnd.github+json",
                "--header",
                "User-Agent: herdr-update-coordinator",
                &url,
            ],
            MAX_PAGE_BYTES,
            seconds + 1,
        )?;
        let entries: Vec<Value> =
            serde_json::from_str(&text).map_err(|e| format!("invalid release catalog: {e}"))?;
        let count = entries.len();
        result.extend(parse_catalog_page(&text)?);
        if count < PAGE_SIZE {
            return Ok(result);
        }
    }
    Ok(result)
}

/// Select the greatest valid SemVer precedence above the running version; equal-precedence
/// duplicates use stable first, then canonical tag and release id for repeatable ordering.
pub fn select_candidate(
    candidates: &[ReleaseCandidate],
    running_version: &str,
    policy: UpdatePolicy,
) -> Result<Option<ReleaseCandidate>, String> {
    let running =
        Version::parse(running_version).map_err(|e| format!("invalid running version: {e}"))?;
    let mut selected: Option<(&ReleaseCandidate, Version)> = None;
    for candidate in candidates {
        candidate.validate()?;
        if !policy.allows_version(&candidate.version)? {
            continue;
        }
        let version = Version::parse(&candidate.version).map_err(|e| e.to_string())?;
        if version.cmp_precedence(&running) != Ordering::Greater {
            continue;
        }
        if selected.as_ref().is_none_or(|(best, previous)| {
            version.cmp_precedence(previous) == Ordering::Greater
                || (version.cmp_precedence(previous) == Ordering::Equal
                    && (candidate.tag.as_str(), candidate.release_id)
                        < (best.tag.as_str(), best.release_id))
        }) {
            selected = Some((candidate, version));
        }
    }
    Ok(selected.map(|(candidate, _)| candidate.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SHA: &str = "a69e52476cc26d14575714dee2b1d1f2513ebd7364a928d3a246e0bfc79dcef4";

    fn fixture(version: &str) -> Value {
        let (_, channel, tag) = classify(version).unwrap();
        let name = format!("HerdrDesktopPet-v{version}-macos-arm64.tar.gz");
        let asset = |name: &str, size: u64| {
            json!({
                "name": name, "size": size, "state": "uploaded",
                "browser_download_url": download_url(&tag, name),
                "digest": format!("sha256:{SHA}")
            })
        };
        json!({
            "tag_name": tag, "id": 17, "draft": false,
            "prerelease": channel == ReleaseChannel::Beta,
            "published_at": "2026-10-09T05:12:01Z",
            "url": format!("{API}/17"),
            "html_url": format!("https://github.com/{REPOSITORY}/releases/tag/{tag}"),
            "assets": [
                asset(&name, 123), asset(&format!("{name}.sha256"), 108),
                asset("SHA256SUMS", 108)
            ]
        })
    }

    fn parse(releases: &[Value]) -> Vec<ReleaseCandidate> {
        parse_catalog_page(&serde_json::to_string(releases).unwrap()).unwrap()
    }

    #[test]
    fn catalog_requires_complete_uploaded_official_assets() {
        let stable = fixture("0.3.3");
        assert_eq!(parse(&[stable.clone()]).len(), 1);
        let mut cases = Vec::new();
        let mut altered = stable.clone();
        altered["draft"] = json!(true);
        cases.push(altered);
        let mut altered = stable.clone();
        altered["url"] = json!("https://api.github.com/repos/attacker/repo/releases/17");
        cases.push(altered);
        let mut altered = stable.clone();
        altered["assets"][0]["browser_download_url"] = json!("https://github.com.evil.test/hanbong5938/herdr-desktop-pet/releases/download/v0.3.3/HerdrDesktopPet-v0.3.3-macos-arm64.tar.gz");
        cases.push(altered);
        let mut altered = stable.clone();
        altered["assets"][0]["browser_download_url"] = json!("https://github.com/hanbong5938/herdr-desktop-pet/releases/download/v0.3.3/../HerdrDesktopPet-v0.3.3-macos-arm64.tar.gz");
        cases.push(altered);
        let mut altered = stable.clone();
        altered["assets"][0]["digest"] = json!("sha256:not-a-digest");
        cases.push(altered);
        let mut altered = stable.clone();
        altered["assets"][0]["state"] = json!("new");
        cases.push(altered);
        let mut altered = stable.clone();
        altered["assets"].as_array_mut().unwrap().pop();
        cases.push(altered);
        let mut altered = stable.clone();
        let duplicate = altered["assets"][0].clone();
        altered["assets"].as_array_mut().unwrap().push(duplicate);
        cases.push(altered);
        let mut altered = stable.clone();
        altered["tag_name"] = json!("v0.3.3+unofficial");
        cases.push(altered);
        let mut altered = stable.clone();
        altered["prerelease"] = json!(true);
        cases.push(altered);
        assert!(parse(&cases).is_empty());
    }

    #[test]
    fn beta_tag_and_checksum_are_bound_to_catalog_digest() {
        let beta = fixture("0.3.3-beta.1");
        let candidate = parse(&[beta.clone()]).pop().unwrap();
        assert_eq!(candidate.tag, "beta/0.3.3-beta.1");
        assert!(candidate
            .validate_checksum(&format!("{SHA}  {}", candidate.asset_name))
            .is_ok());
        assert!(candidate
            .validate_checksum(&format!("{}  {}", "0".repeat(64), candidate.asset_name))
            .is_err());
        let mut altered = beta.clone();
        altered["assets"][0]["browser_download_url"] = json!(download_url(
            "beta%2F0.3.3-beta.1",
            "HerdrDesktopPet-v0.3.3-beta.1-macos-arm64.tar.gz"
        ));
        assert_eq!(parse(&[altered]).len(), 1);
        let mut altered = beta;
        altered["assets"][0]["browser_download_url"] = json!(download_url(
            "beta%252F0.3.3-beta.1",
            "HerdrDesktopPet-v0.3.3-beta.1-macos-arm64.tar.gz"
        ));
        assert!(parse(&[altered]).is_empty());
    }

    #[test]
    fn selection_uses_semver_precedence_without_downgrades() {
        let candidates = parse(&[
            fixture("0.3.3-beta.10"),
            fixture("0.3.3-beta.2"),
            fixture("0.3.3"),
            fixture("0.4.0-beta.1"),
        ]);
        let chosen = |running, policy| {
            select_candidate(&candidates, running, policy)
                .unwrap()
                .map(|candidate| candidate.version)
        };
        assert_eq!(
            chosen("0.3.2", UpdatePolicy::StableOnly).as_deref(),
            Some("0.3.3")
        );
        assert_eq!(
            chosen("0.3.2", UpdatePolicy::StableAndBeta).as_deref(),
            Some("0.4.0-beta.1")
        );
        assert_eq!(
            chosen("0.3.3-beta.2", UpdatePolicy::StableOnly).as_deref(),
            Some("0.3.3")
        );
        let only_betas = parse(&[fixture("0.3.3-beta.2"), fixture("0.3.3-beta.10")]);
        assert_eq!(
            select_candidate(&only_betas, "0.3.2", UpdatePolicy::StableAndBeta)
                .unwrap()
                .unwrap()
                .version,
            "0.3.3-beta.10"
        );
        let same_base = parse(&[fixture("0.3.3-beta.10"), fixture("0.3.3")]);
        assert_eq!(
            select_candidate(&same_base, "0.3.2", UpdatePolicy::StableAndBeta)
                .unwrap()
                .unwrap()
                .version,
            "0.3.3"
        );
        assert_eq!(
            chosen("0.4.0-beta.1+local", UpdatePolicy::StableAndBeta),
            None
        );
        assert_eq!(chosen("0.4.0-beta.1", UpdatePolicy::StableOnly), None);
        assert_eq!(
            UpdatePolicy::StableOnly
                .allows_version("0.3.3-beta.2")
                .unwrap(),
            false
        );
        assert_eq!(
            UpdatePolicy::StableAndBeta
                .allows_version("0.3.3-rc.1")
                .unwrap(),
            false
        );
        assert_eq!(
            UpdatePolicy::StableAndBeta
                .allows_version("0.3.3-beta.10")
                .unwrap(),
            true
        );
        assert_eq!(
            UpdatePolicy::StableAndBeta
                .allows_version("0.3.3+build")
                .unwrap(),
            false
        );
    }
}
