//! The network half of the updater: which URLs may be touched, and how a
//! body arrives safely.
//!
//! Two rules shape this file. First, every URL the updater is handed — by the
//! mirror's metadata or by a redirect — is checked against the same
//! allow-list before a connection is made, because an HTTP client that
//! follows redirects on its own validates only the first hop and then goes
//! wherever it is sent. Second, size caps are applied to bytes as they
//! arrive rather than to a declared `Content-Length`, because the length is
//! a claim by the same party supplying the body.

use http::Uri;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::USER_AGENT;

/// The most hops a redirect chain may have. Long enough for CDNs, short
/// enough that a redirect loop fails in seconds rather than minutes.
const MAX_REDIRECTS: usize = 5;

/// Metadata responses are small; anything larger is not a version list.
pub(super) const MAX_METADATA: u64 = 4 * 1024 * 1024;
/// One archive. Roblox's is around 150 MB across two files; a gigabyte is
/// generous and still bounded.
pub(super) const MAX_ARCHIVE: u64 = 1024 * 1024 * 1024;

/// Co-operative cancellation. Set from another thread; checked between
/// chunks, which is the only place a blocking read can be interrupted. The
/// CLI currently has no way to request a stop, so the setter is unused
/// today but keeps the streaming loop's contract testable.
#[derive(Debug, Default)]
pub(super) struct Cancel(AtomicBool);

impl Cancel {
    #[allow(dead_code)]
    pub(super) fn stop(&self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn check(&self) -> Result<(), Fetch> {
        if self.0.load(std::sync::atomic::Ordering::SeqCst) {
            Err(Fetch::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// A network failure, with the URL attached whenever there is one: "the
/// mirror is down" and "your network is broken" look identical from inside
/// the process, and the URL is what tells them apart.
#[derive(Debug)]
pub(super) enum Fetch {
    Transport { url: String, why: String },
    Status { url: String, status: u16, body: String },
    /// Reached, answered 200, and useless.
    BadData { url: String, why: String },
    Cancelled,
    /// Nothing to do with the network: staging, files.
    Local(String),
}

impl std::fmt::Display for Fetch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport { url, why } => write!(f, "could not reach {url}: {why}"),
            Self::Status { url, status, body } if body.trim().is_empty() => {
                write!(f, "{url} answered HTTP {status}")
            }
            Self::Status { url, status, body } => {
                write!(f, "{url} answered HTTP {status}: {}", body.trim())
            }
            Self::BadData { url, why } => write!(f, "{url} returned unusable data: {why}"),
            Self::Cancelled => f.write_str("the APK download was cancelled"),
            Self::Local(why) => f.write_str(why),
        }
    }
}

/// The hosts a URL may name. A name matches itself and its subdomains: the
/// comparison is on dot-separated labels, because a substring test would let
/// a domain like `evilapkpure.com` satisfy an `apkpure.com` rule.
#[derive(Debug, Clone)]
pub(super) struct Policy {
    hosts: Vec<String>,
}

impl Policy {
    pub(super) fn allowing(hosts: Vec<String>) -> Self {
        Self {
            hosts: hosts
                .into_iter()
                .map(|h| h.trim().trim_end_matches('.').to_ascii_lowercase())
                .filter(|h| !h.is_empty())
                .collect(),
        }
    }

    /// A policy naming only `url`'s own host — used when the user points the
    /// updater at a custom endpoint and says nothing about download hosts.
    pub(super) fn for_url(url: &str) -> Result<Self, Fetch> {
        let host = host_of(url)?;
        Ok(Self::allowing(vec![host]))
    }

    fn permits(&self, host: &str) -> bool {
        self.hosts.iter().any(|allowed| {
            // Same host, or a dot-label under it: the suffix must be
            // preceded by a dot, so `evilapkpure.com` cannot satisfy an
            // `apkpure.com` rule.
            host == allowed
                || host
                    .strip_suffix(allowed)
                    .is_some_and(|rest| rest.ends_with('.'))
        })
    }
}

/// The host of `url`, lowercased. Errors carry the URL rather than looking
/// like a parse failure.
pub(super) fn host_of(url: &str) -> Result<String, Fetch> {
    url.parse::<Uri>()
        .ok()
        .and_then(|u| u.host().map(|h| h.trim_end_matches('.').to_ascii_lowercase()))
        .ok_or_else(|| Fetch::BadData {
            url: url.to_string(),
            why: "this is not a URL with a host in it".into(),
        })
}

/// Reject `url` unless it is a plain https URL to a permitted host.
///
/// [`http::Uri`] does the parsing because strings and parsers disagree on
/// exactly the input that matters: `https://api.pureapk.com@evil.example/`
/// reads to a person as the allowed host while its actual host is
/// `evil.example` — the text before the `@` is a username. No URL this
/// updater fetches has any business carrying credentials, so userinfo is
/// refused outright instead of interpreted.
pub(super) fn check(url: &str, policy: &Policy) -> Result<String, Fetch> {
    let uri: Uri = url
        .parse()
        .map_err(|_| Fetch::BadData {
            url: url.to_string(),
            why: "not a parseable URL".into(),
        })?;

    let scheme = uri.scheme_str().unwrap_or("").to_ascii_lowercase();
    if scheme != "https" {
        return Err(Fetch::BadData {
            url: url.to_string(),
            why: if scheme.is_empty() {
                "the URL has no scheme; only https is allowed".into()
            } else {
                format!("only https is allowed, not {scheme}")
            },
        });
    }
    let authority = uri.authority().ok_or_else(|| Fetch::BadData {
        url: url.to_string(),
        why: "not a parseable URL".into(),
    })?;
    if authority.as_str().contains('@') {
        return Err(Fetch::BadData {
            url: url.to_string(),
            why: "the URL carries credentials, which are never sent".into(),
        });
    }
    if let Some(port) = uri.port_u16()
        && port != 443
    {
        return Err(Fetch::BadData {
            url: url.to_string(),
            why: format!("the URL names port {port}; only 443 is allowed"),
        });
    }
    let host = uri.host().ok_or_else(|| Fetch::BadData {
        url: url.to_string(),
        why: "not a parseable URL".into(),
    })?;
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if !policy.permits(&host) {
        return Err(Fetch::BadData {
            url: url.to_string(),
            why: format!("the host {host} is not on the allow-list for this fetch"),
        });
    }
    Ok(host)
}

/// The client used for every request. Redirects and plain http are disabled
/// at the transport as well as in [`check`], and non-2xx statuses are
/// returned as responses rather than errors: a 302 is not a failure here,
/// and a 503's body may name the reason.
fn agent(connect: Duration, transfer: Duration) -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .https_only(true)
            .user_agent(USER_AGENT)
            .timeout_connect(Some(connect))
            .timeout_global(Some(transfer))
            .build(),
    )
}

/// GET `url`, following redirects by hand and re-checking every target
/// against `policy`. Returns the final URL and whatever response ended the
/// chain, status included.
pub(super) fn get(
    agent: &ureq::Agent,
    url: &str,
    policy: &Policy,
    headers: &[(&str, &str)],
) -> Result<(String, ureq::http::Response<ureq::Body>), Fetch> {
    let mut current = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        check(&current, policy)?;

        let mut request = agent.get(&current);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = request.call().map_err(|e| Fetch::Transport {
            url: current.clone(),
            why: e.to_string(),
        })?;

        let status = response.status().as_u16();
        if !matches!(status, 301 | 302 | 303 | 307 | 308) {
            return Ok((current, response));
        }

        let target = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| Fetch::BadData {
                url: current.clone(),
                why: format!("the host answered {status} with no location to redirect to"),
            })?
            .to_string();
        current = resolve_redirect(&current, target)?;
    }
    Err(Fetch::BadData {
        url: url.to_string(),
        why: format!("redirected more than {MAX_REDIRECTS} times without arriving anywhere"),
    })
}

/// One redirect target, resolved against the hop it came from.
///
/// Origin-relative targets are the only relative form supported: CDNs use
/// them and they cannot move the fetch to another host by construction. An
/// absolute target must already be https; anything else is refused rather
/// than guessed at.
fn resolve_redirect(current: &str, target: String) -> Result<String, Fetch> {
    if target.starts_with("https://") {
        return Ok(target);
    }
    if let Some(rest) = target.strip_prefix('/') {
        let base: Uri = current.parse().map_err(|_| Fetch::BadData {
            url: current.to_string(),
            why: "the redirect could not be resolved against its source URL".into(),
        })?;
        let authority = base.authority().map(|a| a.as_str()).unwrap_or_default();
        return Ok(format!("https://{authority}/{rest}"));
    }
    if let Some((scheme, _)) = target.split_once("://") {
        return Err(Fetch::BadData {
            url: current.to_string(),
            why: format!("the redirect names the scheme {scheme}; only https is followed"),
        });
    }
    Err(Fetch::BadData {
        url: current.to_string(),
        why: "the redirect target is relative and cannot be resolved safely".into(),
    })
}

/// A `GET` that answers with the whole, size-capped body.
pub(super) fn fetch_limited(
    url: &str,
    policy: &Policy,
    headers: &[(&str, &str)],
    cap: u64,
    connect: Duration,
    transfer: Duration,
) -> Result<Vec<u8>, Fetch> {
    let client = agent(connect, transfer);
    let (final_url, mut response) = get(&client, url, policy, headers)?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let body = response
            .body_mut()
            .with_config()
            .limit(64 * 1024)
            .read_to_string()
            .unwrap_or_default();
        return Err(Fetch::Status {
            url: final_url,
            status,
            body,
        });
    }

    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(cap)
        .read_to_end(&mut bytes)
        .map_err(|e| Fetch::Transport {
            url: final_url.clone(),
            why: e.to_string(),
        })?;
    if bytes.len() as u64 >= cap {
        return Err(Fetch::BadData {
            url: final_url,
            why: "the response exceeds its size limit".into(),
        });
    }
    Ok(bytes)
}

/// Stream a URL to `dest`, enforcing the size cap on bytes that arrive and
/// removing the partial file if anything goes wrong.
///
/// The file is written under a `.partial` name and renamed into place only
/// after it is complete and starts with a ZIP local header, so a crash or an
/// HTML error page can never be mistaken for a downloaded APK.
pub(super) fn download_to(
    url: &str,
    policy: &Policy,
    cancel: &Cancel,
    dest: &Path,
    progress: &mut dyn FnMut(u64, Option<u64>),
    connect: Duration,
    transfer: Duration,
) -> Result<u64, Fetch> {
    let client = agent(connect, transfer);
    let (final_url, mut response) = get(&client, url, policy, &[])?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(Fetch::Status {
            url: final_url,
            status,
            body: String::new(),
        });
    }

    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if total.is_some_and(|t| t > MAX_ARCHIVE) {
        return Err(Fetch::BadData {
            url: final_url.clone(),
            why: "the archive declares more bytes than the size limit allows".into(),
        });
    }

    let temporary = dest.with_extension("partial");
    let outcome = (|| -> Result<u64, Fetch> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary)
            .map_err(|e| Fetch::Local(e.to_string()))?;

        let mut reader = response.body_mut().as_reader();
        let mut buf = vec![0u8; 256 * 1024];
        let mut done: u64 = 0;
        let mut announced: u64 = 0;
        loop {
            let n = reader.read(&mut buf).map_err(|e| Fetch::Transport {
                url: final_url.clone(),
                why: e.to_string(),
            })?;
            if n == 0 {
                break;
            }
            done += n as u64;
            cancel.check()?;
            if done > MAX_ARCHIVE {
                return Err(Fetch::BadData {
                    url: final_url.clone(),
                    why: "the archive exceeds its size limit".into(),
                });
            }
            file.write_all(&buf[..n])
                .map_err(|e| Fetch::Local(e.to_string()))?;
            if done - announced >= 1024 * 1024 {
                announced = done;
                progress(done, total);
            }
        }
        if done == 0 {
            return Err(Fetch::BadData {
                url: final_url.clone(),
                why: "the download is empty".into(),
            });
        }
        file.sync_all().map_err(|e| Fetch::Local(e.to_string()))?;
        drop(file);

        // A ZIP local header, checked before anything downstream opens the
        // file: an error page served with status 200 is caught here instead
        // of being reported later as a corrupt archive.
        let mut head = [0u8; 4];
        std::fs::File::open(&temporary)
            .and_then(|mut f| f.read_exact(&mut head))
            .map_err(|e| Fetch::Local(e.to_string()))?;
        if head[0] != b'P' || head[1] != b'K' {
            return Err(Fetch::BadData {
                url: final_url.clone(),
                why: "what arrived is not a ZIP archive, so it is not an APK".into(),
            });
        }

        std::fs::rename(&temporary, dest).map_err(|e| Fetch::Local(e.to_string()))?;
        progress(done, total);
        Ok(done)
    })();

    if outcome.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn permits(list: &[&str], host: &str) -> bool {
        Policy::allowing(list.iter().map(|s| s.to_string()).collect())
            .permits(host)
    }

    #[test]
    fn policy_matches_label_boundaries_not_substrings() {
        assert!(permits(&["apkpure.com"], "apkpure.com"));
        assert!(permits(&["apkpure.com"], "download.apkpure.com"));
        assert!(!permits(&["apkpure.com"], "evilapkpure.com"));
        assert!(!permits(&["apkpure.com"], "apkpure.com.evil.example"));
        // The host arrives lowercased from `check`; this is the last gate,
        // not the normalizer.
        assert!(permits(&["winudf.com"], "winudf.com"));
    }

    #[test]
    fn check_accepts_only_plain_https_to_allowed_hosts() {
        let policy = Policy::allowing(vec!["pureapk.com".into()]);
        assert!(check("https://api.pureapk.com/m/v3", &policy).is_ok());
        assert!(check("http://api.pureapk.com/m/v3", &policy).is_err());
        assert!(check("https://evilpureapk.com/", &policy).is_err());
        // The userinfo before `@` is a username, not a host; the real host
        // is evil.example, which is not on the list — and credentials are
        // refused either way.
        assert!(check("https://api.pureapk.com@evil.example/", &policy).is_err());
        assert!(check("https://api.pureapk.com:8080/", &policy).is_err());
        assert!(check("api.pureapk.com/m/v3", &policy).is_err());
    }

    #[test]
    fn redirects_resolve_origin_relative_targets() {
        let base = "https://api.pureapk.com/m/v3";
        assert_eq!(
            resolve_redirect(base, "https://download.pureapk.com/b/APK/x".into()).unwrap(),
            "https://download.pureapk.com/b/APK/x"
        );
        assert_eq!(
            resolve_redirect(base, "/m/v3?next=1".into()).unwrap(),
            "https://api.pureapk.com/m/v3?next=1"
        );
        assert!(matches!(
            resolve_redirect(base, "http://download.pureapk.com/".into()),
            Err(Fetch::BadData { .. })
        ));
        assert!(matches!(
            resolve_redirect(base, "just/a/path".into()),
            Err(Fetch::BadData { .. })
        ));
    }
}
