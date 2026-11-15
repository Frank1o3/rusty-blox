//! Verifying that Roblox signed an APK.
//!
//! APK Signature Scheme v2 and v3 are Google's published formats, and this
//! module implements the verification side of them from those documents.
//! The check is what makes an untrusted mirror acceptable: whoever served
//! the file can supply anything they like, and the only thing that makes
//! the bytes installable is a valid signature from a certificate whose
//! SHA-256 digest is pinned in `certificates.json`.
//!
//! The three places an implementation goes quietly wrong, and what this
//! one does about each:
//!
//! - **The signing block is not at a fixed offset.** It sits between the
//!   ZIP entries and the central directory, and the End of Central
//!   Directory record itself may be followed by a comment, so both are
//!   found by search rather than by arithmetic.
//! - **The content digest covers the file with one region edited.** The
//!   three covered regions are everything before the signing block, the
//!   central directory, and the EOCD with its "offset of central
//!   directory" field replaced by the signing block's offset. Skip that
//!   substitution and the digest is wrong for every valid archive.
//! - **A signature that parses is not a signature that checks out.** The
//!   signature over the signed data is verified with the signer's own
//!   key, and the content digest is recomputed over the archive and
//!   compared. Both steps are mandatory; either one alone proves nothing.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// `APK Sig Block 42`: the magic that ends every signing block.
const BLOCK_MAGIC: &[u8; 16] = b"APK Sig Block 42";

/// Block ids for the v2 and v3 signature schemes. v3.1 (`0x1b93ad61`) adds
/// key-rotation information for newer platforms; builds carrying it also
/// carry v2 or v3 for older ones, and those are what this verifies.
const BLOCK_ID_V2: u32 = 0x7109_871a;
const BLOCK_ID_V3: u32 = 0xf053_68c0;

/// The v2 attribute stating the highest scheme version the signer also
/// applied. Its absence would leave downgrade attacks open: v2 and v3
/// signatures over the same archive both verify, so an attacker who deletes
/// the v3 block is only defeated by the *signed* v2 data testifying that a
/// v3 block used to exist.
const ATTR_STRIPPING_PROTECTION: u32 = 0xbeef_f00d;

/// Signature algorithm ids the scheme defines and this verifies. Each
/// implies its content digest; the chunked SHA-256 form is the only digest
/// accepted.
const ALG_RSA_PSS_SHA256: u32 = 0x0101;
const ALG_RSA_PKCS1_SHA256: u32 = 0x0103;
const ALG_ECDSA_SHA256: u32 = 0x0201;
const DIGEST_CHUNKED_SHA256: u32 = 1;

/// The chunk size the scheme fixes at one mebibyte. Not tunable.
const CHUNK: usize = 1024 * 1024;

/// A plausible ceiling on signing-block size. Real blocks are a few
/// kilobytes.
const MAX_BLOCK: usize = 64 * 1024 * 1024;

/// Why an archive was not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Not a ZIP, or truncated below the smallest possible one.
    NotAnArchive,
    /// No End of Central Directory record; a truncated download, usually.
    Truncated,
    /// A ZIP, but no APK signing block: unsigned, or v1-only.
    NoSigningBlock,
    /// The block is there and does not parse.
    BadBlock(&'static str),
    /// The block parses and the signature does not verify.
    BadSignature(&'static str),
    /// Correctly signed by a certificate that is not pinned.
    UntrustedCertificate { fingerprint: String },
    /// The v2 signature says a newer scheme was also applied, and is not
    /// present in the file: somebody removed it.
    SchemeStripped { claimed: u32 },
    Io(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnArchive => write!(f, "this file is not a ZIP archive"),
            Self::Truncated => write!(
                f,
                "the archive has no end-of-central-directory record; it is truncated"
            ),
            Self::NoSigningBlock => write!(
                f,
                "the archive carries no APK signing block, so it is unsigned or signed only \
                 with the v1 scheme, which cannot establish where it came from"
            ),
            Self::BadBlock(why) => write!(f, "the archive's signing block does not parse: {why}"),
            Self::BadSignature(why) => {
                write!(f, "the archive's signature does not verify: {why}")
            }
            Self::UntrustedCertificate { fingerprint } => write!(
                f,
                "the archive is correctly signed, but by a certificate rusty-blox does not \
                 trust ({fingerprint})"
            ),
            Self::SchemeStripped { claimed } => write!(
                f,
                "the archive's v2 signature is valid and states that it was also signed with \
                 scheme v{claimed}, but no v{claimed} block is present; somebody removed it to \
                 force this verifier onto the weaker scheme"
            ),
            Self::Io(e) => write!(f, "could not read the archive: {e}"),
        }
    }
}

/// What a verified archive turned out to be signed by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Signer {
    /// SHA-256 over the signing certificate's DER bytes, lowercase hex —
    /// what a pin is compared against and what an error message quotes so
    /// somebody adding a pin can copy it.
    pub(crate) certificate_sha256: String,
}

/// A byte cursor that reports running out instead of panicking.
///
/// Every length in a signing block comes from the file being checked, so
/// every one of them is attacker-chosen.
struct Cursor<'a> {
    buf: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, at: 0 }
    }

    fn remaining(&self) -> usize {
        self.buf.len() - self.at
    }

    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        if end > self.buf.len() {
            return None;
        }
        let s = &self.buf[self.at..end];
        self.at = end;
        Some(s)
    }

    fn u32(&mut self) -> Option<u32> {
        self.bytes(4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    }

    fn u64(&mut self) -> Option<u64> {
        self.bytes(8).map(|b| u64::from_le_bytes(b.try_into().unwrap()))
    }

    /// A length-prefixed sub-sequence, which is how the whole format is
    /// built.
    fn embedded(&mut self) -> Option<Cursor<'a>> {
        let len = self.u32()? as usize;
        self.bytes(len).map(Cursor::new)
    }
}

/// Where the pieces of interest sit in a ZIP.
struct Layout {
    /// Start and end of the APK signing block.
    block: (u64, u64),
    central_directory: u64,
    eocd: u64,
}

/// Locate the EOCD by searching the file's tail (a comment may follow it),
/// then the signing block that ends immediately before the central
/// directory.
fn locate(file: &mut File, len: u64) -> Result<Layout, Refusal> {
    if len < 22 {
        return Err(Refusal::NotAnArchive);
    }
    let window = std::cmp::min(len, 22 + 65535) as usize;
    let start = len - window as u64;
    let mut tail = vec![0u8; window];
    file.seek(SeekFrom::Start(start))
        .and_then(|_| file.read_exact(&mut tail))
        .map_err(|e| Refusal::Io(e.to_string()))?;

    let eocd_rel = (0..=window.saturating_sub(22))
        .rev()
        .find(|&i| tail[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or(Refusal::Truncated)?;
    let eocd = start + eocd_rel as u64;
    let cd_offset = u32::from_le_bytes(
        tail[eocd_rel + 16..eocd_rel + 20]
            .try_into()
            .expect("four bytes"),
    ) as u64;
    if cd_offset >= eocd {
        return Err(Refusal::NotAnArchive);
    }

    // The block's last 24 bytes are an 8-byte size followed by the magic.
    if cd_offset < 24 {
        return Err(Refusal::NoSigningBlock);
    }
    let mut foot = [0u8; 24];
    file.seek(SeekFrom::Start(cd_offset - 24))
        .and_then(|_| file.read_exact(&mut foot))
        .map_err(|e| Refusal::Io(e.to_string()))?;
    if &foot[8..24] != BLOCK_MAGIC {
        return Err(Refusal::NoSigningBlock);
    }
    let size_at_end = u64::from_le_bytes(foot[0..8].try_into().expect("eight bytes"));
    let block_start = cd_offset
        .checked_sub(size_at_end + 8)
        .ok_or(Refusal::BadBlock("the block claims to be larger than the file"))?;
    Ok(Layout {
        block: (block_start, cd_offset),
        central_directory: cd_offset,
        eocd,
    })
}

/// One chunk's digest: `SHA-256(0xa5 || uint32le(len) || chunk)`.
fn chunk_digest(chunk: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update([0xa5]);
    h.update((chunk.len() as u32).to_le_bytes());
    h.update(chunk);
    h.finalize().into()
}

/// The scheme's chunked content digest over the three covered regions.
fn content_digest(file: &mut File, layout: &Layout, len: u64) -> Result<[u8; 32], Refusal> {
    use sha2::{Digest, Sha256};
    let mut chunks: Vec<[u8; 32]> = Vec::new();
    let mut buf = vec![0u8; CHUNK];

    let mut digest_region = |file: &mut File, from: u64, to: u64| -> Result<(), Refusal> {
        let mut at = from;
        while at < to {
            let n = std::cmp::min(CHUNK as u64, to - at) as usize;
            file.seek(SeekFrom::Start(at))
                .and_then(|_| file.read_exact(&mut buf[..n]))
                .map_err(|e| Refusal::Io(e.to_string()))?;
            chunks.push(chunk_digest(&buf[..n]));
            at += n as u64;
        }
        Ok(())
    };

    digest_region(file, 0, layout.block.0)?;
    digest_region(file, layout.central_directory, layout.eocd)?;

    // The EOCD region, with its central-directory offset field patched to
    // point at the signing block before being digested.
    let mut eocd = vec![0u8; (len - layout.eocd) as usize];
    file.seek(SeekFrom::Start(layout.eocd))
        .and_then(|_| file.read_exact(&mut eocd))
        .map_err(|e| Refusal::Io(e.to_string()))?;
    if eocd.len() < 20 {
        return Err(Refusal::Truncated);
    }
    eocd[16..20].copy_from_slice(&(layout.block.0 as u32).to_le_bytes());
    for piece in eocd.chunks(CHUNK) {
        chunks.push(chunk_digest(piece));
    }

    let mut top = Sha256::new();
    top.update([0x5a]);
    top.update((chunks.len() as u32).to_le_bytes());
    for chunk in &chunks {
        top.update(chunk);
    }
    Ok(top.finalize().into())
}

/// Pull the public key bytes and algorithm family out of a DER
/// SubjectPublicKeyInfo. No X.509 parser: the signing block supplies the key
/// on its own, and the less attacker-supplied DER is walked, the better.
fn spki(der: &[u8]) -> Option<(Vec<u8>, bool)> {
    /// One DER tag-length-value, returning (tag, content, rest).
    fn tlv(b: &[u8]) -> Option<(u8, &[u8], &[u8])> {
        let tag = *b.first()?;
        let first = *b.get(1)? as usize;
        let (len, rest) = if first < 0x80 {
            (first, &b[2..])
        } else {
            let n = first & 0x7f;
            if n == 0 || n > 4 || b.len() < 2 + n {
                return None;
            }
            let mut v = 0usize;
            for &byte in &b[2..2 + n] {
                v = (v << 8) | byte as usize;
            }
            (v, &b[2 + n..])
        };
        if rest.len() < len {
            return None;
        }
        Some((tag, &rest[..len], &rest[len..]))
    }

    let (_, seq, _) = tlv(der)?;
    let (_, alg, after_alg) = tlv(seq)?;
    let (_, oid, _) = tlv(alg)?;
    // 1.2.840.113549.1.1.1 (rsaEncryption) and 1.2.840.10045.2.1
    // (id-ecPublicKey).
    let is_rsa = oid == [0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
    let (_, bits, _) = tlv(after_alg)?;
    // A BIT STRING's first content byte is the count of unused trailing bits.
    Some((bits.get(1..)?.to_vec(), is_rsa))
}

/// Verify `sig` over `signed` with the key in `key_der`, for algorithm
/// `alg`.
fn check_signature(alg: u32, key_der: &[u8], signed: &[u8], sig: &[u8]) -> Result<(), Refusal> {
    use ring::signature;
    let (key, is_rsa) =
        spki(key_der).ok_or(Refusal::BadSignature("the signer's public key does not parse"))?;
    let outcome = match (alg, is_rsa) {
        // Roblox's RSA key measures below 2048 bits, so the legacy-size
        // variant is required to check the genuine article at all. The
        // strength of this check is the strength of Roblox's key; the
        // certificate pin is what carries the decision.
        (ALG_RSA_PKCS1_SHA256, true) => signature::UnparsedPublicKey::new(
            &signature::RSA_PKCS1_1024_8192_SHA256_FOR_LEGACY_USE_ONLY,
            &key,
        )
        .verify(signed, sig),
        (ALG_RSA_PSS_SHA256, true) => {
            signature::UnparsedPublicKey::new(&signature::RSA_PSS_2048_8192_SHA256, &key)
                .verify(signed, sig)
        }
        (ALG_ECDSA_SHA256, false) => {
            signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_ASN1, &key)
                .verify(signed, sig)
        }
        // An algorithm this does not know is one nobody has checked it
        // against; guessing would turn an unsupported archive into a
        // silently unverified one.
        _ => {
            return Err(Refusal::BadSignature(
                "the archive is signed with an algorithm rusty-blox does not verify",
            ));
        }
    };
    outcome.map_err(|_| Refusal::BadSignature("the signature does not match the signed data"))
}

/// The stripping-protection attribute in the v2 signed data, if present.
///
/// Optional (older signers omit it); the only intolerance is the attribute
/// claiming a scheme higher than the one actually present, which is a
/// stripped block.
fn stripping_claim(sd: &mut Cursor<'_>) -> Option<u32> {
    let mut attributes = sd.embedded()?;
    while attributes.remaining() > 0 {
        let mut one = attributes.embedded()?;
        if one.u32()? == ATTR_STRIPPING_PROTECTION {
            return one.u32();
        }
    }
    None
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Verify `path` and report which certificate signed it, without consulting
/// any pin. Establishes that the archive is intact and self-consistent;
/// deciding whether that certificate is Roblox's is [`verify_signed_by`]'s
/// job, so "tampered" and "fine but not Roblox's" stay separate answers.
pub(crate) fn verify(path: &Path) -> Result<Signer, Refusal> {
    use sha2::{Digest, Sha256};
    let mut file = File::open(path).map_err(|e| Refusal::Io(e.to_string()))?;
    let len = file
        .metadata()
        .map_err(|e| Refusal::Io(e.to_string()))?
        .len();
    let layout = locate(&mut file, len)?;

    let (start, end) = layout.block;
    let size = (end - start) as usize;
    if size > MAX_BLOCK {
        return Err(Refusal::BadBlock("the block is implausibly large"));
    }
    let mut block = vec![0u8; size];
    file.seek(SeekFrom::Start(start))
        .and_then(|_| file.read_exact(&mut block))
        .map_err(|e| Refusal::Io(e.to_string()))?;

    let mut r = Cursor::new(&block);
    let declared = r.u64().ok_or(Refusal::BadBlock("no leading size"))?;
    if declared + 8 != size as u64 {
        return Err(Refusal::BadBlock("the two size fields disagree"));
    }

    // Walk the id-value pairs. v3 wins over v2 when both are present, which
    // is what the platform does.
    let mut v2: Option<&[u8]> = None;
    let mut v3: Option<&[u8]> = None;
    while r.remaining() > 24 {
        let pair_len = r.u64().ok_or(Refusal::BadBlock("a truncated pair"))? as usize;
        if pair_len < 4 {
            return Err(Refusal::BadBlock("a pair shorter than its own id"));
        }
        let id = r.u32().ok_or(Refusal::BadBlock("a pair with no id"))?;
        let value = r
            .bytes(pair_len - 4)
            .ok_or(Refusal::BadBlock("a pair longer than the block"))?;
        match id {
            BLOCK_ID_V2 => v2 = Some(value),
            BLOCK_ID_V3 => v3 = Some(value),
            _ => {}
        }
    }
    let scheme = v3.or(v2).ok_or(Refusal::NoSigningBlock)?;
    let is_v3 = v3.is_some();

    let mut signers = Cursor::new(
        Cursor::new(scheme)
            .embedded()
            .ok_or(Refusal::BadBlock("no signer sequence"))?
            .buf,
    );
    let signer = signers
        .embedded()
        .ok_or(Refusal::BadBlock("no signers in the block"))?;

    let mut s = signer;
    let signed_data = s.embedded().ok_or(Refusal::BadBlock("no signed data"))?;
    let signed_bytes = signed_data.buf;
    // v3 carries min and max SDK levels between the signed data and the
    // signatures; v2 does not.
    let mut after = s;
    if is_v3 {
        after
            .u32()
            .ok_or(Refusal::BadBlock("no minSdk in the v3 signer"))?;
        after
            .u32()
            .ok_or(Refusal::BadBlock("no maxSdk in the v3 signer"))?;
    }
    let mut signatures = after
        .embedded()
        .ok_or(Refusal::BadBlock("no signatures"))?;
    let public_key = after
        .embedded()
        .ok_or(Refusal::BadBlock("no public key"))?
        .buf;

    // The first signature in a supported algorithm that verifies is proof.
    // Unknown algorithms are skipped, not treated as passes.
    let mut verified = false;
    let mut last = Refusal::BadSignature("no signature used a supported algorithm");
    while signatures.remaining() > 0 {
        let Some(mut one) = signatures.embedded() else {
            break;
        };
        let Some(alg) = one.u32() else {
            break;
        };
        let Some(sig) = one.embedded() else {
            break;
        };
        if std::env::var_os("RUSTY_BLOX_APKSIG_DEBUG").is_some() {
            eprintln!(
                "[apksig] scheme={} alg={alg:#06x} sig={} key={} signed={}",
                if is_v3 { "v3" } else { "v2" },
                sig.buf.len(),
                public_key.len(),
                signed_bytes.len()
            );
        }
        match check_signature(alg, public_key, signed_bytes, sig.buf) {
            Ok(()) => {
                verified = true;
                break;
            }
            Err(e) => last = e,
        }
    }
    if !verified {
        return Err(last);
    }

    // The signed data holds the digests, the certificates and the
    // additional attributes, in that order.
    let mut sd = Cursor::new(signed_bytes);
    let mut digests = sd.embedded().ok_or(Refusal::BadBlock("no digests"))?;
    let mut certificates = sd
        .embedded()
        .ok_or(Refusal::BadBlock("no certificates"))?;

    // The signature algorithm implies its digest; the chunked SHA-256 form
    // is the one this verifier computes.
    let mut expected: Option<Vec<u8>> = None;
    while digests.remaining() > 0 {
        let Some(mut one) = digests.embedded() else {
            break;
        };
        let Some(alg) = one.u32() else {
            break;
        };
        let Some(value) = one.embedded() else {
            break;
        };
        if alg == DIGEST_CHUNKED_SHA256
            || matches!(alg, ALG_RSA_PKCS1_SHA256 | ALG_RSA_PSS_SHA256 | ALG_ECDSA_SHA256)
        {
            expected = Some(value.buf.to_vec());
            break;
        }
    }
    let expected = expected.ok_or(Refusal::BadSignature(
        "the signed data carries no SHA-256 content digest",
    ))?;

    let actual = content_digest(&mut file, &layout, len)?;
    // A mismatch here is the one failure that means the file was changed
    // after signing.
    if expected != actual {
        return Err(Refusal::BadSignature(
            "the archive's contents do not match the digest that was signed",
        ));
    }

    // The additional attributes follow the certificates and are read for
    // the stripping-protection check — only for a v2 verification, since a
    // v3 archive was verified under the newest scheme it carries.
    if !is_v3
        && let Some(claimed) = stripping_claim(&mut sd)
        && claimed > 2
    {
        return Err(Refusal::SchemeStripped { claimed });
    }

    let certificate = certificates
        .embedded()
        .ok_or(Refusal::BadBlock("no signing certificate"))?
        .buf;
    let fingerprint: [u8; 32] = Sha256::digest(certificate).into();
    Ok(Signer {
        certificate_sha256: hex(&fingerprint),
    })
}

/// The certificate digests rusty-blox accepts Roblox builds from.
///
/// Compiled in from `certificates.json` so the binary carries its own trust
/// set; the file lives in the repository rather than a constant in this
/// source so it can be read, diffed and updated without a release. See its
/// header for why adding a digest is a decision rather than a chore.
///
/// `RUSTY_BLOX_TRUSTED_CERTIFICATES` names a file to use instead, for
/// auditing and for the interval between Roblox rotating its key and a
/// release carrying the new digest. Whoever can set that variable can
/// replace the binary anyway, so it widens no boundary that was not already
/// open.
///
/// An unreadable or malformed list yields an empty set, and
/// [`verify_signed_by`] refuses everything against an empty set: trusting
/// nothing stops a download, and trusting everything installs whatever a
/// mirror felt like serving.
pub(crate) fn pinned() -> Vec<String> {
    const BUILT_IN: &str = include_str!("certificates.json");

    let owned;
    let text = match std::env::var_os("RUSTY_BLOX_TRUSTED_CERTIFICATES") {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(t) => {
                owned = t;
                owned.as_str()
            }
            Err(e) => {
                eprintln!(
                    "rusty-blox: could not read the certificate list at {}: {e}",
                    Path::new(&path).display()
                );
                return Vec::new();
            }
        },
        None => BUILT_IN,
    };

    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(text) else {
        eprintln!("rusty-blox: the trusted-certificate list is not valid JSON");
        return Vec::new();
    };

    parsed
        .get("certificates")
        .and_then(|c| c.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|entry| entry.get("sha256").and_then(|v| v.as_str()))
                .map(|d| d.trim().to_ascii_lowercase())
                // A digest that is not 64 hex characters is a typo, and a
                // typo that silently becomes a trusted value is the one
                // mistake this list cannot afford. Dropping it fails closed.
                .filter(|d| d.len() == 64 && d.chars().all(|c| c.is_ascii_hexdigit()))
                .collect()
        })
        .unwrap_or_default()
}

/// Verify `path` and require that its certificate is one of `trusted`.
pub(crate) fn verify_signed_by(path: &Path, trusted: &[String]) -> Result<Signer, Refusal> {
    let signer = verify(path)?;
    if trusted
        .iter()
        .any(|t| t.eq_ignore_ascii_case(&signer.certificate_sha256))
    {
        return Ok(signer);
    }
    Err(Refusal::UntrustedCertificate {
        fingerprint: signer.certificate_sha256,
    })
}

/// Verify a real APK named by `RUSTY_BLOX_VERIFY_APK`.
///
/// Ignored by default because it needs a genuine Roblox APK on disk. This is
/// the strongest check this module has: the whole content digest is
/// recomputed over a 100+ MB archive signed by a key nobody here controls.
/// Run it whenever the verifier or its dependencies change:
///
///     RUSTY_BLOX_VERIFY_APK=~/.local/share/rusty-blox/roblox/base.apk \
///         cargo test apk_update -- --ignored --nocapture
#[cfg(test)]
#[test]
#[ignore = "needs network access and/or a genuine Roblox APK on disk"]
fn verify_real_apk() {
    let Some(path) = std::env::var_os("RUSTY_BLOX_VERIFY_APK") else {
        eprintln!("skipped: set RUSTY_BLOX_VERIFY_APK to a genuine Roblox APK");
        return;
    };
    let path = Path::new(&path);
    let signer = verify(path).expect("verification failed");
    let pinned = pinned();
    eprintln!(
        "fingerprint {} (pinned: {})",
        signer.certificate_sha256,
        pinned.contains(&signer.certificate_sha256)
    );
    assert!(pinned.contains(&signer.certificate_sha256));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ZIP-shaped file: local entries are not needed for these tests, only
    /// an entry region, an optional signing block, a central directory, and
    /// an EOCD — optionally behind a comment — in that order.
    struct Archive {
        entries_len: usize,
        comment: Vec<u8>,
        block: Option<Vec<u8>>,
        /// Corrupt the EOCD's central-directory offset (a layout `locate`
        /// must reject).
        bad_cd_offset: bool,
    }

    impl Archive {
        fn bytes(&self) -> Vec<u8> {
            let mut d = vec![0x50, 0x4b, 0x03, 0x04]; // a local header, for shape
            d.resize(d.len() + self.entries_len, 0xab);
            // The signing block sits between the entries and the central
            // directory; the EOCD's central-directory offset keeps naming
            // the central directory itself, which is where the block's end
            // is found from.
            if let Some(block) = &self.block {
                d.extend_from_slice(block);
            }
            let cd = d.len() as u32;
            // Central directory: a few bytes, enough to be a region.
            d.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02, 0x00, 0x00]);
            let eocd_at = d.len();
            d.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06]);
            d.extend_from_slice(&[0u8; 12]);
            d.extend_from_slice(&cd.to_le_bytes());
            d.extend_from_slice(&0u32.to_le_bytes());
            d.extend_from_slice(&self.comment);
            if self.bad_cd_offset {
                d[eocd_at + 16..eocd_at + 20].copy_from_slice(&u32::MAX.to_le_bytes());
            }
            d
        }

        fn signing_block(&self) -> Vec<u8> {
            // Header pairs, then the trailing size + magic. Starts with its
            // own size field, which `verify` cross-checks.
            let mut pairs = Vec::new();
            // One v2 pair holding four bytes of value.
            let mut pair = 8u64.to_le_bytes().to_vec();
            pair.extend_from_slice(&BLOCK_ID_V2.to_le_bytes());
            pair.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
            pairs.extend_from_slice(&pair);
            let mut out = 8u64.to_le_bytes().to_vec(); // leading size
            out.extend_from_slice(&pairs);
            let size = out.len() as u64 + 8 + 8;
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(BLOCK_MAGIC);
            out[0..8].copy_from_slice(&size.to_le_bytes());
            out
        }
    }

    fn write(dir: &std::path::Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn short_files_and_missing_eocd_are_named() {
        let dir = std::env::temp_dir().join(format!("rusty-blox-apksig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write(&dir, "tiny.apk", &[0x50, 0x4b, 0x03, 0x04]);
        assert!(matches!(verify(&path), Err(Refusal::NotAnArchive)));

        let path = write(&dir, "no-eocd.apk", &[0x50; 64]);
        assert!(matches!(verify(&path), Err(Refusal::Truncated)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zip_without_signing_block_refuses_with_the_v1_explanation() {
        let dir = std::env::temp_dir().join(format!("rusty-blox-apksig2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let archive = Archive {
            entries_len: 16,
            comment: vec![],
            block: None,
            bad_cd_offset: false,
        };
        let path = write(&dir, "plain.apk", &archive.bytes());
        assert!(matches!(verify(&path), Err(Refusal::NoSigningBlock)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn eocd_behind_a_comment_is_still_found() {
        let dir = std::env::temp_dir().join(format!("rusty-blox-apksig3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut archive = Archive {
            entries_len: 256,
            comment: vec![0x21; 300],
            block: None,
            bad_cd_offset: false,
        };
        let path = write(&dir, "commented.apk", &archive.bytes());
        assert!(matches!(verify(&path), Err(Refusal::NoSigningBlock)));

        archive.block = Some(archive.signing_block());
        let path = write(&dir, "commented-block.apk", &archive.bytes());        // The block is found; parsing stops at the bogus pair length inside.
        assert!(matches!(verify(&path), Err(Refusal::BadBlock(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disagreeing_block_sizes_refuse() {
        let dir = std::env::temp_dir().join(format!("rusty-blox-apksig4-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut archive = Archive {
            entries_len: 256,
            comment: vec![],
            block: None,
            bad_cd_offset: false,
        };
        let mut block = archive.signing_block();
        block[0..8].copy_from_slice(&999u64.to_le_bytes());
        archive.block = Some(block);
        let path = write(&dir, "badsize.apk", &archive.bytes());
        assert!(matches!(verify(&path), Err(Refusal::BadBlock(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pinned_ignores_malformed_digests() {
        // Set the override, run, restore. The built-in list has one good
        // digest; a bad override file must yield an empty set.
        let dir = std::env::temp_dir().join(format!("rusty-blox-apksig5-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write(
            &dir,
            "certs.json",
            br#"{"certificates":[{"sha256":"not-a-digest"},{"sha256":"44932ea35a17a267372d71b54d1a0cb3da0dca5113e94406ae2fe18090ba1477"}]}"#,
        );
        unsafe {
            std::env::set_var("RUSTY_BLOX_TRUSTED_CERTIFICATES", &path);
        }
        let pinned = pinned();
        unsafe {
            std::env::remove_var("RUSTY_BLOX_TRUSTED_CERTIFICATES");
        }
        assert_eq!(pinned.len(), 1);
        assert_eq!(
            pinned[0],
            "44932ea35a17a267372d71b54d1a0cb3da0dca5113e94406ae2fe18090ba1477"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
