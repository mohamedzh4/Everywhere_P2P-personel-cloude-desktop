//! Password handshake (CPace-style balanced PAKE on ristretto255) and the
//! message encryption that rides on its key.
//!
//! The same construction is in `web/js/pake.js`; `web/test/vectors.json` pins
//! both to identical bytes. Change one, change both, regenerate the vectors.
//!
//! Protocol (browser = initiator "B", host = responder "H"):
//!   sid      16 random bytes chosen by B
//!   G        = RFC 9496 element derivation of SHA-512(
//!                "everywhere/cpace/v1" || lp(pc_code) || lp(password) || sid)
//!              where lp(x) = 4-byte big-endian length || x
//!   B -> H   Ya = ya * G
//!   H -> B   Yb = yb * G, tag_h
//!   B -> H   tag_b, then data sealed with k_b2h
//!   K        = ya * Yb = yb * Ya   (rejected if it is the identity)
//!   ISK      = SHA-512("everywhere/isk/v1" || sid || Ya || Yb || K)
//!   tag_h    = HMAC-SHA256(ISK, "everywhere/confirm/host")
//!   tag_b    = HMAC-SHA256(ISK, "everywhere/confirm/browser")
//!   k_b2h    = SHA-256("everywhere/key/b2h" || ISK)
//!   k_h2b    = SHA-256("everywhere/key/h2b" || ISK)
//!
//! A wrong password gives different G on each side, so K, ISK and the tags
//! differ, and an eavesdropper cannot test password guesses offline: each guess
//! needs a fresh online attempt, which the host rate-limits.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::traits::Identity;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256, Sha512};

pub const SID_LEN: usize = 16;

fn lp(out: &mut Vec<u8>, x: &[u8]) {
    out.extend_from_slice(&(x.len() as u32).to_be_bytes());
    out.extend_from_slice(x);
}

pub fn generator(pc_code: &str, password: &str, sid: &[u8]) -> RistrettoPoint {
    let mut m = b"everywhere/cpace/v1".to_vec();
    lp(&mut m, pc_code.as_bytes());
    lp(&mut m, password.as_bytes());
    m.extend_from_slice(sid);
    let h: [u8; 64] = Sha512::digest(&m).into();
    RistrettoPoint::from_uniform_bytes(&h)
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("OS random generator");
    b
}

/// Scalar from 64 uniform bytes (random in real use, fixed in test vectors).
pub fn scalar_from_wide(b: &[u8; 64]) -> Scalar {
    Scalar::from_bytes_mod_order_wide(b)
}

pub fn decode_point(b: &[u8]) -> Option<RistrettoPoint> {
    let p = CompressedRistretto::from_slice(b).ok()?.decompress()?;
    (p != RistrettoPoint::identity()).then_some(p)
}

/// Keys both sides end up with.
pub struct SessionKeys {
    pub tag_h: [u8; 32],
    pub tag_b: [u8; 32],
    pub k_b2h: [u8; 32],
    pub k_h2b: [u8; 32],
}

fn hmac(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut m = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("any key length");
    m.update(msg);
    m.finalize().into_bytes().into()
}

pub fn derive(sid: &[u8], ya: &[u8; 32], yb: &[u8; 32], k: &[u8; 32]) -> SessionKeys {
    let mut t = b"everywhere/isk/v1".to_vec();
    t.extend_from_slice(sid);
    t.extend_from_slice(ya);
    t.extend_from_slice(yb);
    t.extend_from_slice(k);
    let isk: [u8; 64] = Sha512::digest(&t).into();
    let kdf = |label: &[u8]| -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(label);
        h.update(isk);
        h.finalize().into()
    };
    SessionKeys {
        tag_h: hmac(&isk, b"everywhere/confirm/host"),
        tag_b: hmac(&isk, b"everywhere/confirm/browser"),
        k_b2h: kdf(b"everywhere/key/b2h"),
        k_h2b: kdf(b"everywhere/key/h2b"),
    }
}

/// Host side: given the browser's Ya, produce Yb and the session keys.
/// `yb_wide` is 64 random bytes (fixed only in tests).
pub fn respond(
    pc_code: &str,
    password: &str,
    sid: &[u8],
    ya_bytes: &[u8],
    yb_wide: &[u8; 64],
) -> Option<([u8; 32], SessionKeys)> {
    let ya_point = decode_point(ya_bytes)?;
    let g = generator(pc_code, password, sid);
    let yb = scalar_from_wide(yb_wide);
    let yb_bytes = (g * yb).compress().to_bytes();
    let k = ya_point * yb;
    if k == RistrettoPoint::identity() {
        return None;
    }
    let ya: [u8; 32] = ya_bytes.try_into().ok()?;
    Some((yb_bytes, derive(sid, &ya, &yb_bytes, &k.compress().to_bytes())))
}

/// Browser side (used by tests and the vector generator; the real browser
/// runs the JS version).
#[cfg_attr(not(test), allow(dead_code))]
pub fn initiate(pc_code: &str, password: &str, sid: &[u8], ya_wide: &[u8; 64]) -> (Scalar, [u8; 32]) {
    let ya = scalar_from_wide(ya_wide);
    let ya_bytes = (generator(pc_code, password, sid) * ya).compress().to_bytes();
    (ya, ya_bytes)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn finish(sid: &[u8], ya: Scalar, ya_bytes: &[u8; 32], yb_bytes: &[u8]) -> Option<SessionKeys> {
    let yb_point = decode_point(yb_bytes)?;
    let k = yb_point * ya;
    if k == RistrettoPoint::identity() {
        return None;
    }
    let yb: [u8; 32] = yb_bytes.try_into().ok()?;
    Some(derive(sid, ya_bytes, &yb, &k.compress().to_bytes()))
}

/// Constant-time comparison for tags.
pub fn tags_equal(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// AES-256-GCM. Output: 12-byte nonce || ciphertext || 16-byte tag.
/// `aad` binds the message to its direction and session id.
pub fn seal(key: &[u8; 32], aad: &[u8], plaintext: &[u8], nonce: [u8; 12]) -> Vec<u8> {
    let c = Aes256Gcm::new_from_slice(key).expect("32-byte key");
    let ct = c
        .encrypt(&Nonce::from(nonce), Payload { msg: plaintext, aad })
        .expect("encryption cannot fail for valid sizes");
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    out
}

pub fn open(key: &[u8; 32], aad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < 12 + 16 {
        return None;
    }
    let c = Aes256Gcm::new_from_slice(key).ok()?;
    let nonce: [u8; 12] = sealed[..12].try_into().ok()?;
    c.decrypt(&Nonce::from(nonce), Payload { msg: &sealed[12..], aad }).ok()
}

pub fn aad(direction: &str, sid: &[u8]) -> Vec<u8> {
    let mut a = format!("everywhere/{direction}/").into_bytes();
    a.extend_from_slice(sid);
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Vectors {
        pc_code: String,
        password: String,
        sid: String,
        ya_wide: String,
        yb_wide: String,
        ya: String,
        yb: String,
        tag_h: String,
        tag_b: String,
        k_b2h: String,
        k_h2b: String,
        seal_nonce: String,
        seal_plain: String,
        seal_b2h: String,
    }

    fn h<const N: usize>(s: &str) -> [u8; N] {
        hex::decode(s).unwrap().try_into().unwrap()
    }

    const VECTORS: &str = include_str!("../../web/test/vectors.json");

    #[test]
    fn matches_shared_vectors() {
        let v: Vectors = serde_json::from_str(VECTORS).unwrap();
        let sid = hex::decode(&v.sid).unwrap();
        let (ya, ya_bytes) = initiate(&v.pc_code, &v.password, &sid, &h(&v.ya_wide));
        assert_eq!(hex::encode(ya_bytes), v.ya);
        let (yb_bytes, hk) = respond(&v.pc_code, &v.password, &sid, &ya_bytes, &h(&v.yb_wide)).unwrap();
        assert_eq!(hex::encode(yb_bytes), v.yb);
        let bk = finish(&sid, ya, &ya_bytes, &yb_bytes).unwrap();
        for k in [&hk, &bk] {
            assert_eq!(hex::encode(k.tag_h), v.tag_h);
            assert_eq!(hex::encode(k.tag_b), v.tag_b);
            assert_eq!(hex::encode(k.k_b2h), v.k_b2h);
            assert_eq!(hex::encode(k.k_h2b), v.k_h2b);
        }
        let sealed = seal(&bk.k_b2h, &aad("b2h", &sid), v.seal_plain.as_bytes(), h(&v.seal_nonce));
        assert_eq!(hex::encode(&sealed), v.seal_b2h);
        assert_eq!(open(&hk.k_b2h, &aad("b2h", &sid), &sealed).unwrap(), v.seal_plain.as_bytes());
    }

    #[test]
    fn wrong_password_gives_different_keys() {
        let sid = [7u8; 16];
        let (ya, ya_bytes) = initiate("482913007", "right horse", &sid, &[1u8; 64]);
        let (yb_bytes, hk) = respond("482913007", "wrong horse", &sid, &ya_bytes, &[2u8; 64]).unwrap();
        let bk = finish(&sid, ya, &ya_bytes, &yb_bytes).unwrap();
        assert!(!tags_equal(&hk.tag_h, &bk.tag_h));
        assert!(!tags_equal(&hk.tag_b, &bk.tag_b));
        let sealed = seal(&bk.k_b2h, &aad("b2h", &sid), b"offer", [0; 12]);
        assert!(open(&hk.k_b2h, &aad("b2h", &sid), &sealed).is_none());
    }

    #[test]
    fn rejects_bad_points_and_tampering() {
        assert!(decode_point(&[0u8; 32]).is_none()); // identity
        assert!(decode_point(&[0xffu8; 32]).is_none()); // not canonical
        assert!(decode_point(&[1u8; 5]).is_none());
        let key = [9u8; 32];
        let mut s = seal(&key, b"a", b"hello", [3; 12]);
        s[20] ^= 1;
        assert!(open(&key, b"a", &s).is_none());
        let s = seal(&key, b"a", b"hello", [3; 12]);
        assert!(open(&key, b"b", &s).is_none(), "aad must bind the message");
    }

    /// Prints fresh vectors. Run: cargo test print_vectors -- --ignored --nocapture
    #[test]
    #[ignore]
    fn print_vectors() {
        let (pc, pw) = ("482913007", "correct horse battery");
        let sid: [u8; 16] = core::array::from_fn(|i| i as u8);
        let ya_wide: [u8; 64] = core::array::from_fn(|i| (i * 3 + 1) as u8);
        let yb_wide: [u8; 64] = core::array::from_fn(|i| (i * 5 + 2) as u8);
        let (ya, ya_bytes) = initiate(pc, pw, &sid, &ya_wide);
        let (yb_bytes, k) = respond(pc, pw, &sid, &ya_bytes, &yb_wide).unwrap();
        let _ = finish(&sid, ya, &ya_bytes, &yb_bytes).unwrap();
        let nonce: [u8; 12] = core::array::from_fn(|i| (100 + i) as u8);
        let plain = "v=0 sdp offer";
        let sealed = seal(&k.k_b2h, &aad("b2h", &sid), plain.as_bytes(), nonce);
        let out = serde_json::json!({
            "pc_code": pc, "password": pw, "sid": hex::encode(sid),
            "ya_wide": hex::encode(ya_wide), "yb_wide": hex::encode(yb_wide),
            "ya": hex::encode(ya_bytes), "yb": hex::encode(yb_bytes),
            "tag_h": hex::encode(k.tag_h), "tag_b": hex::encode(k.tag_b),
            "k_b2h": hex::encode(k.k_b2h), "k_h2b": hex::encode(k.k_h2b),
            "seal_nonce": hex::encode(nonce), "seal_plain": plain, "seal_b2h": hex::encode(sealed),
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    }
}
