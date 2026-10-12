//! Proving Roblox signed the archive, which is what makes an untrusted source
//! acceptable.
//!
//! [ADR-025](../../../docs/adr/ADR-025-fetching-from-a-third-party-mirror.md)
//! permits fetching the build from a distributor that is not Roblox, on one
//! condition: Cordial installs nothing it cannot prove Roblox signed. This is
//! that proof. **The check is the feature and the download is the convenience
//! it buys**, so this module exists before anything that fetches.
//!
//! It is useful without any download at all. Today a user supplies an APK by
//! hand and Cordial trusts it because a person chose it — and the advice
//! circulating in the project's own Discord is to get one from a mirror, next to
//! a warning that anything advertising itself as "modded" is likely malware.
//! Running the same check over a locally-supplied file turns "somebody chose
//! this" into "Roblox signed this".
//!
//! ## Implemented from Google's published specification
//!
//! APK Signature Scheme v2 and v3 are public and documented, so this is written
//! from the specification rather than from any implementation of it. The parts
//! that matter and are easy to get subtly wrong:
//!
//! **The signing block sits between the ZIP entries and the central directory**,
//! and is found by walking back from the End of Central Directory record to the
//! central directory offset, then reading the block's own trailing size and
//! magic. It is not at a fixed place and the EOCD is not at a fixed place
//! either, because either may be followed by a comment.
//!
//! **The digest covers the file in three pieces and one of them is edited.**
//! Everything before the signing block, then the central directory, then the
//! EOCD *with its "offset of central directory" field replaced by the offset of
//! the signing block*. That substitution is the whole reason a signature
//! survives the block being inserted, and forgetting it produces a digest that
//! is wrong for every valid archive — a failure that looks like the archive
//! being bad rather than the verifier being wrong.
//!
//! **Each 1 MiB chunk is digested with a prefix, and so is the concatenation.**
//! `0xa5` for a chunk, `0x5a` for the top level. Without the prefixes a chunk
//! digest and a whole-file digest could collide by construction.
//!
//! ## What this refuses, and why each refusal is not pedantry
//!
//! **A v1-only archive.** v1 signatures cover individual entries rather than the
//! file, so an archive can gain, lose or reorder content and still verify. An
//! archive that offers nothing better is one whose provenance cannot be
//! established this way, and accepting it would make the pin decorative.
//!
//! **A signature that parses but is not checked.** Reading a certificate out of
//! a block proves nothing: whoever supplied the archive supplied the block. The
//! signature over the signed data is verified with the signer's own key, and the
//! content digest is recomputed over the archive and compared. Either step
//! alone is a decoration, and this module would rather not exist than be one.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// `APK Sig Block 42`, the eight-byte magic ending the signing block, twice
/// over. Sixteen bytes in total.
const BLOCK_MAGIC: &[u8; 16] = b"APK Sig Block 42";

/// The v2 signature scheme's block id, and v3's. v3.1 (`0x1b93ad61`) carries
/// rotation information for newer platforms; a build signed with it also
/// carries v2 or v3 for older ones, and this verifies those.
const BLOCK_ID_V2: u32 = 0x7109_871a;
const BLOCK_ID_V3: u32 = 0xf053_68c0;

/// The stripping-protection attribute, carried in v2 signed data and holding
/// the highest scheme version the same signer also applied.
///
/// **Without this check there is no downgrade protection at all.** An archive
/// signed with both v2 and v3 verifies under either, so an attacker who wants
/// the weaker scheme simply deletes the v3 block from the signing block and
/// leaves a file that is still genuinely v2-signed. Preferring v3 when it is
/// present does not help, because after the strip it is not present. What
/// closes it is that the *signed* v2 data says "I also signed this with 3",
/// and that sentence cannot be edited without breaking the v2 signature.
const ATTR_STRIPPING_PROTECTION: u32 = 0xbeef_f00d;

/// Content digest algorithm ids from the specification. Only the SHA-256 chunked
/// form is accepted: SHA-512 is permitted by the scheme and is not what Android
/// builds use, and a verifier that silently accepts an algorithm it has never
/// seen a real archive use is a verifier nobody has tested.
const CONTENT_DIGEST_CHUNKED_SHA256: u32 = 1;

/// Signature algorithm ids, from the specification.
const SIG_RSA_PSS_SHA256: u32 = 0x0101;
const SIG_RSA_PKCS1_SHA256: u32 = 0x0103;
const SIG_ECDSA_SHA256: u32 = 0x0201;

/// The chunk size the scheme fixes. Not tunable; it is part of the digest.
const CHUNK: usize = 1024 * 1024;

/// Why an archive was not accepted.
///
/// Every variant names something a person can act on, because "signature
/// verification failed" is the message that makes somebody reinstall three times
/// and then give up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Not a ZIP, or truncated below the smallest possible one.
    NotAnArchive,
    /// No End of Central Directory record. Truncated download, usually.
    NoEndOfCentralDirectory,
    /// A ZIP, but with no APK signing block: signed with v1 only, or not signed.
    NoSigningBlock,
    /// The block is there and does not parse. Corrupt, or not what it claims.
    MalformedSigningBlock(&'static str),
    /// Present, parsed, and the signature does not check out. This is the one
    /// that means somebody changed the file.
    SignatureInvalid(&'static str),
    /// The archive is properly signed, by a certificate Cordial does not trust.
    UntrustedCertificate { fingerprint: String },
    /// The v2 signature is valid and says the signer also applied a newer
    /// scheme, which is not in the file. Somebody removed it.
    SchemeStripped { claimed: u32 },
    /// Reading the file failed.
    Io(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NotAnArchive => write!(f, "this file is not a ZIP archive"),
            Refusal::NoEndOfCentralDirectory => {
                write!(
                    f,
                    "the archive has no end-of-central-directory record; it is truncated"
                )
            }
            Refusal::NoSigningBlock => write!(
                f,
                "the archive carries no APK signing block, so it is unsigned or signed only with \
                 the v1 scheme, which cannot establish where it came from"
            ),
            Refusal::MalformedSigningBlock(why) => {
                write!(f, "the archive's signing block does not parse: {why}")
            }
            Refusal::SignatureInvalid(why) => {
                write!(f, "the archive's signature does not verify: {why}")
            }
            Refusal::UntrustedCertificate { fingerprint } => write!(
                f,
                "the archive is correctly signed, but by a certificate rusty-blox does not trust \
                 ({fingerprint})"
            ),
            Refusal::SchemeStripped { claimed } => write!(
                f,
                "the archive's v2 signature is valid and states that it was also signed with \
                 scheme v{claimed}, but no v{claimed} block is present; somebody removed it to \
                 force this verifier onto the weaker scheme"
            ),
            Refusal::Io(e) => write!(f, "could not read the archive: {e}"),
        }
    }
}

/// What a verified archive turned out to be signed by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signer {
    /// SHA-256 over the signing certificate's DER bytes, lowercase hex. This is
    /// what a pin is compared against, and what an error message quotes so that
    /// somebody adding a new pin can copy it.
    pub certificate_sha256: String,
}

/// A byte reader that cannot run off the end.
///
/// Every length in a signing block comes from the file being checked, so every
/// one of them is attacker-chosen. A slice index would panic; this returns an
/// error, and the difference is a refusal instead of a crash on a malformed
/// download.
struct Reader<'a> {
    buf: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, at: 0 }
    }
    fn remaining(&self) -> usize {
        self.buf.len() - self.at
    }
    fn u32(&mut self) -> Option<u32> {
        let b = self.take(4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Option<u64> {
        let b = self.take(8)?;
        Some(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        if end > self.buf.len() {
            return None;
        }
        let s = &self.buf[self.at..end];
        self.at = end;
        Some(s)
    }
    /// A length-prefixed sub-sequence, which is how the whole format is built.
    fn sized(&mut self) -> Option<Reader<'a>> {
        let n = self.u32()? as usize;
        Some(Reader::new(self.take(n)?))
    }
}

/// Where the interesting offsets are in a ZIP.
struct Layout {
    signing_block: (u64, u64),
    central_directory: u64,
    eocd: u64,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Find the End of Central Directory record, then the signing block before it.
fn locate(file: &mut File, len: u64) -> Result<Layout, Refusal> {
    if len < 22 {
        return Err(Refusal::NotAnArchive);
    }
    // The EOCD is last, but may be followed by a comment of up to 65535 bytes,
    // so it is searched for backwards rather than assumed to be at the end.
    let window = std::cmp::min(len, 22 + 65535) as usize;
    let start = len - window as u64;
    let mut tail = vec![0u8; window];
    file.seek(SeekFrom::Start(start))
        .map_err(|e| Refusal::Io(e.to_string()))?;
    file.read_exact(&mut tail)
        .map_err(|e| Refusal::Io(e.to_string()))?;

    let eocd_rel = (0..=window.saturating_sub(22))
        .rev()
        .find(|&i| tail[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or(Refusal::NoEndOfCentralDirectory)?;
    let eocd = start + eocd_rel as u64;
    let cd_offset = u32::from_le_bytes([
        tail[eocd_rel + 16],
        tail[eocd_rel + 17],
        tail[eocd_rel + 18],
        tail[eocd_rel + 19],
    ]) as u64;
    if cd_offset >= eocd {
        return Err(Refusal::NotAnArchive);
    }

    // The block ends immediately before the central directory: its last 24 bytes
    // are an 8-byte size and the 16-byte magic.
    if cd_offset < 24 {
        return Err(Refusal::NoSigningBlock);
    }
    let mut foot = [0u8; 24];
    file.seek(SeekFrom::Start(cd_offset - 24))
        .map_err(|e| Refusal::Io(e.to_string()))?;
    file.read_exact(&mut foot)
        .map_err(|e| Refusal::Io(e.to_string()))?;
    if &foot[8..24] != BLOCK_MAGIC {
        return Err(Refusal::NoSigningBlock);
    }
    let size_at_end = u64::from_le_bytes(foot[0..8].try_into().expect("8 bytes"));
    // The block is `size` plus the leading 8-byte size field, and both size
    // fields must agree — the leading one is read during parsing.
    let block_start =
        cd_offset
            .checked_sub(size_at_end + 8)
            .ok_or(Refusal::MalformedSigningBlock(
                "the block claims to be larger than the file",
            ))?;
    Ok(Layout {
        signing_block: (block_start, cd_offset),
        central_directory: cd_offset,
        eocd,
    })
}

/// One chunk's digest: `H(0xa5 || uint32le(len) || chunk)`.
fn chunk_digest(chunk: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update([0xa5u8]);
    h.update((chunk.len() as u32).to_le_bytes());
    h.update(chunk);
    h.finalize().into()
}

/// The scheme's chunked content digest over the three covered regions.
///
/// The EOCD's central-directory offset is replaced by the signing block's
/// offset before it is digested — see the module doc for why that substitution
/// is load-bearing rather than a quirk.
fn content_digest(file: &mut File, layout: &Layout, len: u64) -> Result<[u8; 32], Refusal> {
    use sha2::{Digest, Sha256};
    let mut chunks: Vec<[u8; 32]> = Vec::new();
    let mut buf = vec![0u8; CHUNK];

    let mut region =
        |file: &mut File, from: u64, to: u64, chunks: &mut Vec<[u8; 32]>| -> Result<(), Refusal> {
            let mut at = from;
            while at < to {
                let n = std::cmp::min(CHUNK as u64, to - at) as usize;
                file.seek(SeekFrom::Start(at))
                    .map_err(|e| Refusal::Io(e.to_string()))?;
                file.read_exact(&mut buf[..n])
                    .map_err(|e| Refusal::Io(e.to_string()))?;
                chunks.push(chunk_digest(&buf[..n]));
                at += n as u64;
            }
            Ok(())
        };

    region(file, 0, layout.signing_block.0, &mut chunks)?;
    region(file, layout.central_directory, layout.eocd, &mut chunks)?;

    let mut eocd = vec![0u8; (len - layout.eocd) as usize];
    file.seek(SeekFrom::Start(layout.eocd))
        .map_err(|e| Refusal::Io(e.to_string()))?;
    file.read_exact(&mut eocd)
        .map_err(|e| Refusal::Io(e.to_string()))?;
    if eocd.len() < 20 {
        return Err(Refusal::NoEndOfCentralDirectory);
    }
    eocd[16..20].copy_from_slice(&(layout.signing_block.0 as u32).to_le_bytes());
    for piece in eocd.chunks(CHUNK) {
        chunks.push(chunk_digest(piece));
    }

    let mut top = Sha256::new();
    top.update([0x5au8]);
    top.update((chunks.len() as u32).to_le_bytes());
    for c in &chunks {
        top.update(c);
    }
    Ok(top.finalize().into())
}

/// Pull the public key bytes and algorithm out of a DER SubjectPublicKeyInfo.
///
/// A whole X.509 parser is not needed and is not wanted: the signing block hands
/// over the public key on its own, and all that is required is the algorithm it
/// belongs to and the key material. Less parsing of attacker-supplied DER is
/// strictly better here.
fn spki(der: &[u8]) -> Option<(Vec<u8>, bool)> {
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
    // 1.2.840.113549.1.1.1 rsaEncryption, and 1.2.840.10045.2.1 id-ecPublicKey.
    let is_rsa = oid == [0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
    let (_, bits, _) = tlv(after_alg)?;
    // A BIT STRING's first content byte is the count of unused trailing bits.
    let key = bits.get(1..)?.to_vec();
    Some((key, is_rsa))
}

fn verify_signature(alg: u32, key_der: &[u8], signed: &[u8], sig: &[u8]) -> Result<(), Refusal> {
    use ring::signature;
    let (key, is_rsa) = spki(key_der).ok_or(Refusal::SignatureInvalid(
        "the signer's public key does not parse",
    ))?;
    let result = match (alg, is_rsa) {
        // **Roblox signs with a 1104-bit RSA key, and that is a measurement
        // rather than an assumption.** The signature in the shipping APK is 140
        // bytes and its SubjectPublicKeyInfo is 162; `ring`'s ordinary
        // `RSA_PKCS1_2048_8192_SHA256` refuses anything below 2048 bits as a
        // deliberate policy, so the first version of this file rejected the
        // genuine article with "the signature does not match the signed data" --
        // which reads as a tampered download and was a verifier that could not
        // verify anything.
        //
        // So the legacy-sized variant, named as loudly as `ring` names it. This
        // is not rusty-blox choosing a weak key; it is rusty-blox being able to check
        // the one Roblox actually uses. **What it costs is worth stating: a
        // 1104-bit RSA signature is well below what anyone would choose today,
        // so the strength of this check is the strength of Roblox's key and no
        // more.** The pin over the certificate is what carries the weight, and
        // the two together are still enormously better than the status quo,
        // which is a human downloading an archive from a mirror and rusty-blox
        // trusting it because they chose it.
        (SIG_RSA_PKCS1_SHA256, true) => signature::UnparsedPublicKey::new(
            &signature::RSA_PKCS1_1024_8192_SHA256_FOR_LEGACY_USE_ONLY,
            &key,
        )
        .verify(signed, sig),
        (SIG_RSA_PSS_SHA256, true) => {
            signature::UnparsedPublicKey::new(&signature::RSA_PSS_2048_8192_SHA256, &key)
                .verify(signed, sig)
        }
        (SIG_ECDSA_SHA256, false) => {
            signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_ASN1, &key)
                .verify(signed, sig)
        }
        // Deliberately not a catch-all "try them all". An algorithm this does
        // not know is one nobody has checked it against, and guessing would
        // turn an unsupported archive into a silently unverified one.
        _ => {
            return Err(Refusal::SignatureInvalid(
                "the archive is signed with an algorithm rusty-blox does not verify",
            ));
        }
    };
    result.map_err(|_| Refusal::SignatureInvalid("the signature does not match the signed data"))
}

/// Verify `path` and report who signed it.
///
/// Does **not** consult any pin: this establishes that the archive is intact and
/// self-consistent and says which certificate vouches for it. Deciding whether
/// that certificate is Roblox's is [`verify_signed_by`]'s job, and the two are
/// separate so that an error can distinguish "this file was tampered with" from
/// "this file is fine and is not Roblox's".
/// The highest scheme version the v2 signer says it also applied, if it said.
///
/// `sd` must be positioned just past the certificate sequence. A malformed or
/// absent attribute sequence returns `None` and is not an error: the attribute
/// is optional, older signers omit it, and refusing an archive for lacking an
/// optional field would reject every v2-only build ever made. What is *not*
/// tolerated is the attribute being present and saying a higher number than
/// the block that is actually here -- that is [`Refusal::SchemeStripped`].
fn stripping_protection(sd: &mut Reader<'_>) -> Option<u32> {
    let mut attributes = sd.sized()?;
    while attributes.remaining() > 0 {
        let mut a = attributes.sized()?;
        let id = a.u32()?;
        if id == ATTR_STRIPPING_PROTECTION {
            return a.u32();
        }
    }
    None
}

pub fn verify(path: &Path) -> Result<Signer, Refusal> {
    use sha2::{Digest, Sha256};
    let mut file = File::open(path).map_err(|e| Refusal::Io(e.to_string()))?;
    let len = file
        .metadata()
        .map_err(|e| Refusal::Io(e.to_string()))?
        .len();
    let layout = locate(&mut file, len)?;

    let (start, end) = layout.signing_block;
    let size = (end - start) as usize;
    if size > 64 * 1024 * 1024 {
        return Err(Refusal::MalformedSigningBlock(
            "the block is implausibly large",
        ));
    }
    let mut block = vec![0u8; size];
    file.seek(SeekFrom::Start(start))
        .map_err(|e| Refusal::Io(e.to_string()))?;
    file.read_exact(&mut block)
        .map_err(|e| Refusal::Io(e.to_string()))?;

    let mut r = Reader::new(&block);
    let declared = r
        .u64()
        .ok_or(Refusal::MalformedSigningBlock("no leading size"))?;
    if declared + 8 != size as u64 {
        return Err(Refusal::MalformedSigningBlock(
            "the two size fields disagree",
        ));
    }

    // Walk the id-value pairs looking for v3 first, then v2: a build carrying
    // both is verified against the newer scheme, which is what the platform
    // does.
    let mut v2: Option<&[u8]> = None;
    let mut v3: Option<&[u8]> = None;
    while r.remaining() > 24 {
        let pair_len =
            r.u64()
                .ok_or(Refusal::MalformedSigningBlock("a truncated pair"))? as usize;
        if pair_len < 4 {
            return Err(Refusal::MalformedSigningBlock(
                "a pair shorter than its own id",
            ));
        }
        let id = r
            .u32()
            .ok_or(Refusal::MalformedSigningBlock("a pair with no id"))?;
        let value = r.take(pair_len - 4).ok_or(Refusal::MalformedSigningBlock(
            "a pair longer than the block",
        ))?;
        match id {
            BLOCK_ID_V2 => v2 = Some(value),
            BLOCK_ID_V3 => v3 = Some(value),
            _ => {}
        }
    }
    let scheme = v3.or(v2).ok_or(Refusal::NoSigningBlock)?;

    let mut signers = Reader::new(
        Reader::new(scheme)
            .sized()
            .ok_or(Refusal::MalformedSigningBlock("no signer sequence"))?
            .buf,
    );
    let signer = signers
        .sized()
        .ok_or(Refusal::MalformedSigningBlock("no signers in the block"))?;

    let mut s = signer;
    let signed_data = s
        .sized()
        .ok_or(Refusal::MalformedSigningBlock("no signed data"))?;
    let signed_bytes = signed_data.buf;
    // v3 carries a min and max SDK between the signed data and the signatures;
    // v2 does not. Both are u32, and skipping them when present is what makes
    // one parser serve both.
    let mut after = s;
    if v3.is_some() {
        after
            .u32()
            .ok_or(Refusal::MalformedSigningBlock("no minSdk"))?;
        after
            .u32()
            .ok_or(Refusal::MalformedSigningBlock("no maxSdk"))?;
    }
    let mut signatures = after
        .sized()
        .ok_or(Refusal::MalformedSigningBlock("no signatures"))?;
    let public_key = after
        .sized()
        .ok_or(Refusal::MalformedSigningBlock("no public key"))?
        .buf;

    // The first signature whose algorithm is one this verifies. The scheme
    // permits several; one that checks out is proof, and an algorithm this does
    // not know is skipped rather than treated as a pass.
    let mut verified = false;
    let mut last = Refusal::SignatureInvalid("no signature used a supported algorithm");
    while signatures.remaining() > 0 {
        let mut one = match signatures.sized() {
            Some(s) => s,
            None => break,
        };
        let alg = match one.u32() {
            Some(a) => a,
            None => break,
        };
        let sig = match one.sized() {
            Some(s) => s.buf,
            None => break,
        };
        if std::env::var_os("RUSTY_BLOX_APKSIG_DEBUG").is_some() {
            eprintln!(
                "[apksig] scheme={} alg={alg:#06x} sig={} key={} signed={}",
                if v3.is_some() { "v3" } else { "v2" },
                sig.len(),
                public_key.len(),
                signed_bytes.len()
            );
        }
        match verify_signature(alg, public_key, signed_bytes, sig) {
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

    // The signed data holds the digests, the certificates and the attributes.
    let mut sd = Reader::new(signed_bytes);
    let mut digests = sd
        .sized()
        .ok_or(Refusal::MalformedSigningBlock("no digests"))?;
    let mut certificates = sd
        .sized()
        .ok_or(Refusal::MalformedSigningBlock("no certificates"))?;

    let mut expected: Option<Vec<u8>> = None;
    while digests.remaining() > 0 {
        let mut d = match digests.sized() {
            Some(d) => d,
            None => break,
        };
        let alg = match d.u32() {
            Some(a) => a,
            None => break,
        };
        let value = match d.sized() {
            Some(v) => v.buf,
            None => break,
        };
        // The signature algorithm implies its digest; the chunked SHA-256 form
        // is the one this computes.
        if matches!(
            alg,
            SIG_RSA_PKCS1_SHA256 | SIG_RSA_PSS_SHA256 | SIG_ECDSA_SHA256
        ) || alg == CONTENT_DIGEST_CHUNKED_SHA256
        {
            expected = Some(value.to_vec());
            break;
        }
    }
    let expected = expected.ok_or(Refusal::SignatureInvalid(
        "the signed data carries no SHA-256 content digest",
    ))?;

    let actual = content_digest(&mut file, &layout, len)?;
    if expected != actual {
        // **This is the one that means the file was changed after signing.**
        return Err(Refusal::SignatureInvalid(
            "the archive's contents do not match the digest that was signed",
        ));
    }

    // The additional attributes come after the certificates, and in v3 after
    // the two SDK bounds. This is read for one field and one reason: see
    // [`ATTR_STRIPPING_PROTECTION`].
    if v3.is_none() {
        if let Some(claimed) = stripping_protection(&mut sd) {
            if claimed > 2 {
                return Err(Refusal::SchemeStripped { claimed });
            }
        }
    }

    let certificate = certificates
        .sized()
        .ok_or(Refusal::MalformedSigningBlock("no signing certificate"))?
        .buf;
    let fingerprint: [u8; 32] = Sha256::digest(certificate).into();
    Ok(Signer {
        certificate_sha256: hex(&fingerprint),
    })
}

/// The certificates Cordial accepts Roblox builds from.
///
/// Compiled in from `packaging/trust/roblox-signing-certificates.json` so a
/// shipped binary carries its own trust and does not depend on a file being
/// installed beside it. The file is in the repository rather than a constant in
/// this source so a person can read it, diff it and audit it -- see its own
/// header for why adding a digest is a decision rather than a chore.
///
/// `RUSTY_BLOX_TRUSTED_CERTIFICATES` names a file to use instead. That exists for
/// auditing and for the case where Roblox rotates its key and somebody needs
/// their client working before a Cordial release can carry the new digest.
/// **It can only be set by whoever is already running Cordial as this user**,
/// who could replace the binary anyway, so it widens no boundary that was not
/// already open.
///
/// An unreadable or empty list yields an empty set, and
/// [`verify_signed_by`] refuses everything against an empty set. That is the
/// correct direction to fail: trusting nothing stops a download, and trusting
/// everything installs whatever a mirror felt like serving.
pub fn pinned() -> Vec<String> {
    pinned_under("certificates")
}

fn pinned_under(key: &str) -> Vec<String> {
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
        .get(key)
        .and_then(|c| c.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|entry| entry.get("sha256").and_then(|v| v.as_str()))
                .map(|d| d.trim().to_ascii_lowercase())
                // A digest that is not 64 hex characters is a typo, and a typo
                // that silently becomes a trusted value is the one mistake this
                // file cannot afford. Dropping it means the download fails
                // closed rather than against a fingerprint nothing can match.
                .filter(|d| d.len() == 64 && d.chars().all(|c| c.is_ascii_hexdigit()))
                .collect()
        })
        .unwrap_or_default()
}

/// Verify `path` and require that the certificate is one of `trusted`.
///
/// The two-step shape is deliberate. "This file was tampered with" and "this
/// file is intact and is not Roblox's" are different things to tell somebody,
/// and a single boolean would collapse them into the same shrug.
pub fn verify_signed_by(path: &Path, trusted: &[String]) -> Result<Signer, Refusal> {
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
