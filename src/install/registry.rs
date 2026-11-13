//! The published npm release is Jecode's one artifact distribution: both
//! npm and standalone updates read it. Metadata comes from the registry over
//! HTTPS; the tarball must match the registry's `sha512` integrity.

use crate::cancel::Cancellation;
use crate::json::{self, Value};
use std::time::Duration;

pub const PACKAGE: &str = "@giovannijecha/jecode";
const DEFAULT_REGISTRY: &str = "https://registry.npmjs.org";
const METADATA_LIMIT: usize = 16 * 1024 * 1024;
const TARBALL_LIMIT: usize = 256 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub tarball: String,
    pub sha512: Vec<u8>,
}

/// `JECODE_UPDATE_REGISTRY` selects a mirror or a test registry.
pub fn registry() -> String {
    std::env::var("JECODE_UPDATE_REGISTRY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_REGISTRY.into())
        .trim_end_matches('/')
        .to_owned()
}

fn get(url: &str, accept: &str, limit: usize) -> Result<Vec<u8>, String> {
    let cancel = Cancellation::default();
    let mut decoder = crate::http::Decoder::default();
    let mut body = Vec::new();
    let output = crate::http::run(
        crate::http::Request {
            url,
            method: "GET",
            headers: &[("Accept", accept), ("User-Agent", "jecode-update")],
            body: None,
            timeout: Some(Duration::from_secs(600)),
            idle: Some(Duration::from_secs(60)),
            output_limit: 0,
        },
        &cancel,
        &mut |bytes| {
            decoder.feed(bytes, &mut |part| {
                if body.len() + part.len() > limit {
                    return Err(format!("{url} returned more than {} MiB", limit >> 20));
                }
                body.extend_from_slice(part);
                Ok(())
            })
        },
    )?;
    if output.timed_out || output.exit_code != Some(0) || output.input_error.is_some() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(format!(
            "Could not reach {url}{}",
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        ));
    }
    match decoder.status {
        Some(200..=299) => Ok(body),
        Some(status) => Err(format!("{url} answered HTTP {status}")),
        None => Err(format!("{url} sent no HTTP status")),
    }
}

/// The newest published release, from the registry's `latest` tag.
pub fn latest() -> Result<Release, String> {
    let registry = registry();
    let url = format!("{registry}/{}", PACKAGE.replace('/', "%2f"));
    let body = get(
        &url,
        "application/vnd.npm.install-v1+json; q=1.0, application/json; q=0.8",
        METADATA_LIMIT,
    )?;
    let text = String::from_utf8(body).map_err(|_| "Registry metadata is not UTF-8")?;
    parse(&json::parse(&text)?, &registry)
}

pub fn parse(document: &Value, registry: &str) -> Result<Release, String> {
    let version = document
        .get("dist-tags")
        .and_then(|tags| tags.get("latest"))
        .and_then(Value::as_str)
        .ok_or("Registry metadata has no latest version")?;
    Version::parse(version).ok_or("Registry metadata has an invalid latest version")?;
    let dist = document
        .get("versions")
        .and_then(|versions| versions.get(version))
        .and_then(|entry| entry.get("dist"))
        .ok_or("Registry metadata lacks the latest version's distribution")?;
    let tarball = dist
        .get("tarball")
        .and_then(Value::as_str)
        .ok_or("Registry metadata lacks a tarball URL")?;
    if registry.starts_with("https://") && !tarball.starts_with("https://") {
        return Err("The registry offered a tarball without HTTPS; nothing was downloaded".into());
    }
    let integrity = dist
        .get("integrity")
        .and_then(Value::as_str)
        .ok_or("The release has no integrity digest; nothing was downloaded")?;
    let sha512 = integrity
        .split_whitespace()
        .find_map(|entry| entry.strip_prefix("sha512-"))
        .ok_or("The release has no sha512 integrity digest; nothing was downloaded")
        .and_then(|encoded| {
            crate::attachments::base64::decode(encoded)
                .map_err(|_| "The release integrity digest is malformed")
        })?;
    if sha512.len() != 64 {
        return Err("The release integrity digest is malformed".into());
    }
    Ok(Release {
        version: version.to_owned(),
        tarball: tarball.to_owned(),
        sha512,
    })
}

/// The tarball, accepted only when its SHA-512 matches the registry's.
pub fn download(release: &Release) -> Result<Vec<u8>, String> {
    let bytes = get(&release.tarball, "application/octet-stream", TARBALL_LIMIT)?;
    verify(&bytes, &release.sha512)?;
    Ok(bytes)
}

pub fn verify(bytes: &[u8], sha512: &[u8]) -> Result<(), String> {
    if super::sha512::digest(bytes).as_slice() != sha512 {
        return Err(
            "The downloaded release does not match the registry's sha512 integrity; nothing was installed"
                .into(),
        );
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    core: (u64, u64, u64),
    /// A release sorts after its prereleases.
    release: bool,
    pre: String,
}

impl Version {
    pub fn parse(text: &str) -> Option<Self> {
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let core = core.split('+').next()?;
        let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
        let version = (parts.next()??, parts.next()??, parts.next()??);
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            core: version,
            release: pre.is_none(),
            pre: pre.unwrap_or_default().to_owned(),
        })
    }
}

pub fn newer(candidate: &str, current: &str) -> bool {
    match (Version::parse(candidate), Version::parse(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opt-in: the published release, verified as `jecode update` does.
    #[test]
    #[ignore = "contacts the public npm registry"]
    fn live_published_release_verifies_and_extracts() {
        let release = latest().unwrap();
        let tarball = download(&release).unwrap();
        let started = std::time::Instant::now();
        let binary = super::super::standalone::executable(&tarball, &release).unwrap();
        eprintln!(
            "{} · {} bytes compressed · {} bytes executable · extracted in {:?}",
            release.version,
            tarball.len(),
            binary.len(),
            started.elapsed()
        );
        assert!(binary.len() > 1024 * 1024);
    }

    #[test]
    fn versions_order_releases_after_prereleases() {
        assert!(newer("0.7.6", "0.7.5"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(newer("1.0.0", "1.0.0-beta.1"));
        assert!(!newer("0.7.5", "0.7.5"));
        assert!(!newer("0.7.4", "0.7.5"));
        assert!(!newer("garbage", "0.7.5"));
    }

    #[test]
    fn metadata_needs_https_and_a_sha512_digest() {
        let digest = crate::attachments::base64::encode(&[1u8; 64]);
        let document = |tarball: &str, integrity: &str| {
            json::parse(&format!(
                r#"{{"dist-tags":{{"latest":"9.9.9"}},"versions":{{"9.9.9":{{"dist":{{"tarball":"{tarball}","integrity":"{integrity}"}}}}}}}}"#
            ))
            .unwrap()
        };
        let release = parse(
            &document(
                "https://registry.example/j.tgz",
                &format!("sha512-{digest}"),
            ),
            "https://registry.example",
        )
        .unwrap();
        assert_eq!(release.version, "9.9.9");
        assert_eq!(release.sha512, vec![1u8; 64]);
        assert!(
            parse(
                &document("http://registry.example/j.tgz", &format!("sha512-{digest}")),
                "https://registry.example"
            )
            .is_err()
        );
        assert!(
            parse(
                &document("https://registry.example/j.tgz", "sha1-abc"),
                "https://registry.example"
            )
            .unwrap_err()
            .contains("sha512")
        );
        assert!(verify(b"payload", &[0u8; 64]).is_err());
        assert!(verify(b"payload", &super::super::sha512::digest(b"payload")).is_ok());
    }
}
