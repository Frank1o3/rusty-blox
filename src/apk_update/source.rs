//! The APKPure source: which build is newest, and how its archives arrive.
//!
//! APKPure publishes no API contract. Its version endpoint answers with a
//! length-delimited binary encoding that is protocol-buffer-shaped but has
//! no published schema, so the reader below works from byte patterns that
//! have held across every response measured so far and treats a mismatch as
//! an error rather than a guess. What makes this acceptable at all happens
//! after the download: `signature` refuses anything Roblox did not sign, so
//! a reader mistake fails the update instead of installing the wrong build.
//!
//! Wire facts (last verified 2026-08-26 against `2.734.917`, and 2026-09-24
//! for the ABI notes):
//!
//! - `GET` on the metadata URL with headers `x-cv`, `x-sv`, `x-gp` and an
//!   `x-abis` filter answers 200 with roughly 100 KB of binary.
//! - Version records contain, back to back: the version code as ASCII
//!   digits after a `0x2a` field tag, the version name as ASCII after
//!   `0x32`, and a `0x3a` byte closing the name.
//! - A record's download URL follows an `APKJ` (or `XAPKJ`) marker: two
//!   bytes of length varint, then the URL as ASCII.
//! - Asking with `x-abis: x86_64` specifically returns one monolithic APK
//!   carrying every ABI. The literal string matters: asking with the host's
//!   own ABI on non-x86 hosts returns XAPK bundles this updater cannot
//!   open, so the filter is never substituted with the runtime ABI.
//! - The x86_64-filtered list is not always complete for older versions, so
//!   a version missing from it is retried once with every ABI name joined.

use super::net::{self, Cancel, Fetch, Policy};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The endpoint APKPure serves the version list from.
const METADATA_URL: &str =
    "https://api.pureapk.com/m/v3/cms/app_version?hl=en-US&package_name=com.roblox.client";

/// The hosts APKPure's own metadata and CDN use. `winudf.com` is where the
/// download bytes actually come from, so a list without it refuses every
/// real transfer.
const DOWNLOAD_HOSTS: [&str; 3] = ["pureapk.com", "apkpure.com", "winudf.com"];

/// The APKPure client version the response shape is keyed on. When the
/// reader stops finding version records in a healthy 200 response, this
/// constant is the first thing to check.
const HEADER_CV: &str = "3172501";
/// Claimed Android SDK level, and an opaque flag the real client sends.
const HEADER_SV: &str = "29";
const HEADER_GP: &str = "1";

/// The ABI filter that selects the monolithic bundle, whatever host the
/// client runs on — see the module comment.
const ABI_FILTER: &str = "x86_64";
/// Every ABI, for the retry on older versions.
const ABI_FILTER_ALL: &str = "arm64-v8a,armeabi-v7a,armeabi,x86,x86_64";

/// More than this many distinct candidate archives for one version means the
/// response was not understood; downloading them all to find out would cost
/// gigabytes.
const MAX_CANDIDATES: usize = 4;

const CONNECT: Duration = Duration::from_secs(10);
const METADATA_TIMEOUT: Duration = Duration::from_secs(60);
const ARCHIVE_TIMEOUT: Duration = Duration::from_secs(900);

/// Override the metadata endpoint (`RUSTY_BLOX_APK_MIRROR_URL`) and the
/// hosts that may serve downloads (`RUSTY_BLOX_APK_MIRROR_HOSTS`, comma
/// separated).
///
/// A custom URL without an explicit host list is allowed its own host and
/// subdomains and nothing else. Whatever the endpoint, the signature check
/// still gates installation: a hostile mirror can waste bandwidth and see
/// who asked, and nothing else.
fn policy() -> Result<(String, Policy), Fetch> {
    let Some(url) = std::env::var("RUSTY_BLOX_APK_MIRROR_URL")
        .ok()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
    else {
        return Ok((
            METADATA_URL.to_string(),
            Policy::allowing(DOWNLOAD_HOSTS.iter().map(|s| s.to_string()).collect()),
        ));
    };

    // Validate the configured URL now, when a typo can name the variable in
    // the error, rather than later as a transport failure.
    let host = net::host_of(&url)?;
    net::check(&url, &Policy::for_url(&url)?)?;
    let hosts = match std::env::var("RUSTY_BLOX_APK_MIRROR_HOSTS")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        Some(list) => list.split(',').map(|h| h.trim().to_string()).collect(),
        None => vec![host],
    };
    Ok((url, Policy::allowing(hosts)))
}

/// What the updater tells its caller about as it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Progress {
    Asking {
        endpoint: &'static str,
    },
    Fetch {
        file: String,
        done: u64,
        total: Option<u64>,
    },
    Verify {
        file: String,
    },
}

/// A build the mirror offers: display name (`2.734.917`) and version code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Available {
    pub(crate) name: String,
    #[allow(dead_code)]
    pub(crate) code: u64,
}

/// The downloaded archives, named by what they contain rather than by what
/// the mirror called them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Archives {
    pub(crate) base: PathBuf,
    pub(crate) split: PathBuf,
}

impl Archives {
    /// The distinct files to verify: a monolithic archive is both `base` and
    /// `split` and is checked once.
    pub(crate) fn distinct(&self) -> Vec<&Path> {
        if self.base == self.split {
            vec![self.base.as_path()]
        } else {
            vec![self.base.as_path(), self.split.as_path()]
        }
    }
}

/// GET the version list with `abi` as the filter.
fn metadata(endpoint: &str, abi: &str) -> Result<Vec<u8>, Fetch> {
    let headers: Vec<(&str, String)> = vec![
        ("x-cv", HEADER_CV.to_string()),
        ("x-sv", HEADER_SV.to_string()),
        ("x-gp", HEADER_GP.to_string()),
        ("x-abis", abi.to_string()),
    ];
    let borrowed: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let metadata_policy = Policy::for_url(endpoint)?;
    net::fetch_limited(
        endpoint,
        &metadata_policy,
        &borrowed,
        net::MAX_METADATA,
        CONNECT,
        METADATA_TIMEOUT,
    )
}

/// One version record recovered from the metadata blob.
#[derive(Debug)]
struct Record {
    name: String,
    code: u64,
    url: String,
}

/// Bytes allowed in a version name (`2.734.917`) and in a download URL.
fn name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'_' | b'-')
}

fn url_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-@:%._+~#=?&/()".contains(&b)
}

/// Every version record in the response, in file order.
///
/// The record grammar is a contiguous run: `0x2a`, length, ASCII digits
/// (the code), `0x32`, length, version-name bytes, `0x3a`. Requiring all of
/// it to be adjacent is what keeps one record's code from being attached to
/// another record's name, which a scan for names alone would allow.
///
/// A record's own download URL is found in a second pass, over the bytes
/// between one record's name and the next record's grammar match.
fn records(d: &[u8]) -> Vec<Record> {
    // Pass one: the record boundaries.
    let mut spans: Vec<(usize, String, u64)> = Vec::new();
    let mut i = 0usize;
    while i + 2 < d.len() {
        if d[i] != 0x2a {
            i += 1;
            continue;
        }
        let code_len = d[i + 1] as usize;
        let Some(code) = d.get(i + 2..i + 2 + code_len) else {
            i += 1;
            continue;
        };
        if code.is_empty() || code.len() > 20 || !code.iter().all(u8::is_ascii_digit) {
            i += 1;
            continue;
        }
        let after_code = i + 2 + code_len;
        if d.get(after_code) != Some(&0x32) {
            i += 1;
            continue;
        }
        let name_len = match d.get(after_code + 1) {
            Some(&n) if (1..=30).contains(&n) => n as usize,
            _ => {
                i += 1;
                continue;
            }
        };
        let Some(name) = d.get(after_code + 2..after_code + 2 + name_len) else {
            i += 1;
            continue;
        };
        if !name.iter().all(|&b| name_byte(b)) || !name.contains(&b'.') {
            i += 1;
            continue;
        }
        let name_end = after_code + 2 + name_len;
        if d.get(name_end) != Some(&0x3a) {
            i += 1;
            continue;
        }
        let mut code_value: u64 = 0;
        let mut overflow = false;
        for b in code {
            code_value = match code_value
                .checked_mul(10)
                .and_then(|v| v.checked_add((b - b'0') as u64))
            {
                Some(v) => v,
                None => {
                    overflow = true;
                    break;
                }
            };
        }
        if overflow || code_value == 0 {
            i += 1;
            continue;
        }
        spans.push((
            name_end,
            String::from_utf8_lossy(name).into_owned(),
            code_value,
        ));
        i = name_end;
    }

    // Pass two: one download URL per record.
    let mut out = Vec::with_capacity(spans.len());
    for (index, (name_end, name, code)) in spans.iter().enumerate() {
        let end = spans
            .get(index + 1)
            .map(|(next, _, _)| *next)
            .unwrap_or(d.len());
        let Some(url) = url_in(&d[name_end + 1..end]) else {
            continue;
        };
        out.push(Record {
            name: name.clone(),
            code: *code,
            url,
        });
    }
    out
}

/// The first download URL in one record's body.
///
/// The URL follows an `APKJ` (or `XAPKJ`) marker — marker, then a two-byte
/// length varint — and runs until a byte that cannot appear in one of these
/// URLs.
fn url_in(body: &[u8]) -> Option<String> {
    let mut at = 0usize;
    while at + 6 <= body.len() {
        let found = body[at..].windows(4).position(|w| w == b"APKJ")?;
        let start = at + found + 6;
        let mut stop = start;
        while stop < body.len() && url_byte(body[stop]) {
            stop += 1;
        }
        if let Ok(candidate) = std::str::from_utf8(&body[start..stop])
            && candidate.starts_with("https://")
        {
            return Some(candidate.to_string());
        }
        at += found + 4;
    }
    None
}

/// The newest version the metadata offers.
///
/// Asked with the narrow filter on purpose: the newest x86_64-filtered
/// entry is a build this client can actually run. Widening the filter for
/// this query would let an ARM-only release become "the newest version",
/// which measured worse than it sounds — the ARM release it once surfaced
/// carried no x86_64 split in either of its bundles, so every later step
/// would have worked towards an engine that could not start.
fn newest(progress: &mut dyn FnMut(Progress)) -> Result<Available, Fetch> {
    progress(Progress::Asking { endpoint: "apkpure" });
    let (endpoint, _download_policy) = policy()?;
    let body = metadata(&endpoint, ABI_FILTER)?;
    let all = records(&body);
    let record = all.first().ok_or_else(|| Fetch::BadData {
        url: endpoint.to_string(),
        why: "the version list carries no versions. If RUSTY_BLOX_APK_MIRROR_URL points at a \
              custom endpoint it may be answering in a shape this reader does not understand; \
              otherwise the service has changed"
            .into(),
    })?;
    Ok(Available {
        name: record.name.clone(),
        code: record.code,
    })
}

/// Download `version`'s archives into the empty directory `into`.
///
/// Candidates are tried in the order the metadata offers them and the loop
/// stops as soon as one archive has been shown to hold both the engine and
/// the assets — the ordinary case is a single monolithic APK, and fetching
/// the remaining candidates after that lands would multiply the transfer
/// for no information.
fn fetch(
    version: &Available,
    cancel: &Cancel,
    into: &Path,
    progress: &mut dyn FnMut(Progress),
) -> Result<Archives, Fetch> {
    if into
        .read_dir()
        .map(|mut d| d.next().is_some())
        .unwrap_or(false)
    {
        return Err(Fetch::Local(format!(
            "{} already holds files; a download needs an empty staging directory so a \
             previous attempt's leftovers cannot be mistaken for this one's",
            into.display()
        )));
    }

    let (endpoint, download_policy) = policy()?;
    let mut urls = downloads_for(&metadata(&endpoint, ABI_FILTER)?, &version.name);
    if urls.is_empty() {
        // The filtered list is not reliably complete for older versions;
        // retry with every ABI. This cannot buy a wrong answer because what
        // arrives is opened and checked for the engine below.
        urls = downloads_for(&metadata(&endpoint, ABI_FILTER_ALL)?, &version.name);
    }
    if urls.is_empty() {
        return Err(Fetch::BadData {
            url: endpoint.to_string(),
            why: format!("the mirror does not offer version {}", version.name),
        });
    }
    if urls.len() > MAX_CANDIDATES {
        return Err(Fetch::BadData {
            url: endpoint.to_string(),
            why: format!(
                "the mirror offers {} different archives for version {}, which is more than \
                 this updater will try",
                urls.len(),
                version.name
            ),
        });
    }
    for url in &urls {
        net::check(url, &download_policy)?;
    }

    let mut landed: Vec<PathBuf> = Vec::new();
    let outcome = (|| -> Result<(), Fetch> {
        for (index, url) in urls.iter().enumerate() {
            let dest = into.join(format!("candidate-{index}.apk"));
            let name = dest
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let mut report = |done: u64, total: Option<u64>| {
                progress(Progress::Fetch {
                    file: name.clone(),
                    done,
                    total,
                })
            };
            net::download_to(
                url,
                &download_policy,
                cancel,
                &dest,
                &mut report,
                CONNECT,
                ARCHIVE_TIMEOUT,
            )?;
            landed.push(dest);
            if classify(&landed).is_some() {
                return Ok(());
            }
        }
        Ok(())
    })();

    if let Err(e) = outcome {
        // No partial candidate set survives a failure: leftovers a later run
        // mistakes for a complete download are how assets from one release
        // end up beside an engine from another.
        for path in &landed {
            let _ = std::fs::remove_file(path);
        }
        return Err(e);
    }

    classify(&landed).ok_or_else(|| Fetch::BadData {
        url: endpoint.to_string(),
        why: format!(
            "the mirror served {} archive(s) for version {} and none of them carries {}, so \
             there is no engine in what arrived",
            landed.len(),
            version.name,
            super::LIBRARY_IN_APK
        ),
    })
}

/// The download URLs the metadata offers for exactly `version`.
fn downloads_for(d: &[u8], version: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for record in records(d) {
        if record.name == version && !out.contains(&record.url) {
            out.push(record.url);
        }
    }
    out
}

/// Which of `files` holds the engine, and which holds the assets.
///
/// Archives are read rather than trusted by name. A monolithic APK holding
/// both answers both from one file.
fn classify(files: &[PathBuf]) -> Option<Archives> {
    let mut engine: Option<&PathBuf> = None;
    let mut assets: Option<&PathBuf> = None;
    for file in files {
        let Ok(handle) = std::fs::File::open(file) else {
            continue;
        };
        let Ok(mut zip) = zip::ZipArchive::new(std::io::BufReader::new(handle)) else {
            continue;
        };
        if engine.is_none() && zip.by_name(super::LIBRARY_IN_APK).is_ok() {
            engine = Some(file);
        }
        if assets.is_none()
            && (0..zip.len()).any(|i| {
                zip.by_index(i)
                    .map(|entry| {
                        entry
                            .name()
                            .map(|name| name.starts_with("assets/"))
                            .unwrap_or(false)
                    })
                    .unwrap_or(false)
            })
        {
            assets = Some(file);
        }
        if engine.is_some() && assets.is_some() {
            break;
        }
    }
    let engine = engine?;
    Some(Archives {
        base: assets.cloned().unwrap_or_else(|| engine.clone()),
        split: engine.clone(),
    })
}

#[derive(Debug, Default)]
pub(crate) struct ApkPure;

impl ApkPure {
    pub(crate) fn newest(&self, progress: &mut dyn FnMut(Progress)) -> Result<Available, Fetch> {
        newest(progress)
    }

    pub(crate) fn fetch(
        &self,
        version: &Available,
        cancel: &Cancel,
        into: &Path,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Archives, Fetch> {
        fetch(version, cancel, into, progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A version record in the shape the endpoint answers with: code after
    /// `0x2a`, name after `0x32`, closed by `0x3a`, then a `0x4a` field
    /// carrying an `APKJ` marker, a two-byte length, and the URL.
    fn record(code: &str, name: &str, url: &str) -> Vec<u8> {
        let mut d = Vec::new();
        d.push(0x2a);
        d.push(code.len() as u8);
        d.extend_from_slice(code.as_bytes());
        d.push(0x32);
        d.push(name.len() as u8);
        d.extend_from_slice(name.as_bytes());
        d.push(0x3a);
        d.extend_from_slice(b"APKJ");
        let url_len = url.len();
        d.push((url_len >> 8) as u8);
        d.push((url_len & 0xff) as u8);
        d.extend_from_slice(url.as_bytes());
        d
    }

    #[test]
    fn records_find_codes_names_and_urls() {
        let mut d = Vec::new();
        d.extend_from_slice(&record("2908", "2.734.917", "https://download.pureapk.com/b/a1"));
        d.extend_from_slice(&record("2904", "2.734.916", "https://download.pureapk.com/b/a2"));
        d.push(0x00);

        let found = records(&d);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "2.734.917");
        assert_eq!(found[0].code, 2908);
        assert_eq!(found[0].url, "https://download.pureapk.com/b/a1");
        assert_eq!(found[1].code, 2904);
    }

    #[test]
    fn records_reject_disconnected_code_and_name() {
        // A code field and a name field that are not adjacent must not be
        // joined into a record.
        let mut d = record("", "", "");
        d.clear();
        d.extend_from_slice(&[0x2a, 2]);
        d.extend_from_slice(b"99");
        d.extend_from_slice(&[0x00, 0x00, 0x00]);
        d.extend_from_slice(&[0x32, 9]);
        d.extend_from_slice(b"2.734.917");
        d.push(0x3a);
        assert!(records(&d).is_empty());
    }

    #[test]
    fn downloads_for_returns_one_url_per_matching_record() {
        let mut d = Vec::new();
        d.extend_from_slice(&record("2908", "2.734.917", "https://download.pureapk.com/b/a1"));
        d.extend_from_slice(&record("2908", "2.734.917", "https://download.pureapk.com/b/a1"));
        d.extend_from_slice(&record("2904", "2.734.916", "https://download.pureapk.com/b/a2"));
        let urls = downloads_for(&d, "2.734.917");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0], "https://download.pureapk.com/b/a1");
        assert!(downloads_for(&d, "9.9.9").is_empty());
    }

    #[test]
    fn classify_identifies_engine_and_assets_by_contents() {
        let dir = std::env::temp_dir().join(format!("rusty-blox-classify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let engine = dir.join("engine.apk");
        let assets = dir.join("assets.apk");
        write_zip(&engine, &[(super::super::LIBRARY_IN_APK, b"engine")]);
        write_zip(&assets, &[("assets/some.bin", b"data")]);

        let classified = classify(&[engine.clone(), assets.clone()]).unwrap();
        assert_eq!(classified.split, engine);
        assert_eq!(classified.base, assets);
        // A monolithic archive is both.
        let both = dir.join("both.apk");
        write_zip(
            &both,
            &[(super::super::LIBRARY_IN_APK, b"engine"), ("assets/some.bin", b"data")],
        );
        let classified = classify(&[both.clone()]).unwrap();
        assert_eq!(classified.base, both);
        assert_eq!(classified.split, both);
        assert_eq!(classified.distinct().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options =
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in entries {
            zip.start_file(*name, options).unwrap();
            std::io::Write::write_all(&mut zip, body).unwrap();
        }
        zip.finish().unwrap();
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// Ask the live endpoint which build is newest, without downloading.
    ///
    /// Ignored by default: it needs the network, and it is a probe against a
    /// third-party service, not a test of code under this repository's
    /// control. Run it when the metadata reader or its constants change:
    ///
    ///     cargo test apk_update -- --ignored --nocapture
    #[test]
    #[ignore = "needs network access to APKPure"]
    fn newest_live() {
        let mut progress = |_item: Progress| {};
        let available = newest(&mut progress).expect("the live metadata probe failed");
        eprintln!(
            "newest x86-64 build: {} (code {})",
            available.name, available.code
        );
        assert!(available.name.starts_with("2."), "unexpected version shape");
    }
}
