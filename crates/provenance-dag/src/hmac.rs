// SPDX-License-Identifier: UNLICENSED
//! **HMAC-SHA256 (RFC 2104), écrit à la main sur [`crate::sha256`] — ZÉRO dépendance.**
//!
//! C'est la primitive de scellement : chaque maillon de la chaîne du DAG est un `HMAC(clé, …)`. Prouvé
//! par les vecteurs **RFC 4231**. Comme SHA-256, il est ici en clair pour être *trustless*.

use crate::sha256::sha256;

const BLOCK: usize = 64; // taille de bloc de SHA-256 (octets)

/// **HMAC-SHA256** de `msg` sous la clé `key` (RFC 2104).
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    // Normalisation de clé : si > bloc, on la hache ; sinon on la bourre de zéros à droite.
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&sha256(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }

    // inner = SHA256(ipad ‖ msg)
    let mut inner = Vec::with_capacity(BLOCK + msg.len());
    inner.extend_from_slice(&ipad);
    inner.extend_from_slice(msg);
    let inner_hash = sha256(&inner);

    // HMAC = SHA256(opad ‖ inner)
    let mut outer = Vec::with_capacity(BLOCK + 32);
    outer.extend_from_slice(&opad);
    outer.extend_from_slice(&inner_hash);
    sha256(&outer)
}

/// Comparaison **à temps constant** de deux tranches (anti-timing sur la vérification de sceau).
/// Renvoie `true` ssi même longueur et mêmes octets, sans court-circuit dépendant du contenu.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::{ct_eq, hmac_sha256};

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn rfc4231_vectors() {
        // RFC 4231 — vecteurs de test HMAC-SHA-256.
        // Test 1 : clé 0x0b×20, données "Hi There".
        assert_eq!(
            hex(&hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        // Test 2 : clé "Jefe", données "what do ya want for nothing?".
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Test 4 : clé 0x01..0x19 (25 octets), données 0xcd×50.
        let key4: Vec<u8> = (1u8..=25).collect();
        assert_eq!(
            hex(&hmac_sha256(&key4, &[0xcd; 50])),
            "82558a389a443c0ea4cc819899f2083a85f0faa3e578f8077a2e3ff46729665b"
        );
    }

    #[test]
    fn long_key_is_hashed() {
        // Clé > 64 octets : doit être hachée d'abord (RFC 4231 test 6 exerce ce chemin).
        let key: Vec<u8> = vec![0xaa; 131];
        let mac = hmac_sha256(&key, b"Test Using Larger Than Block-Size Key - Hash Key First");
        assert_eq!(hex(&mac), "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54");
    }

    #[test]
    fn ct_eq_behaves() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
        assert!(ct_eq(b"", b""));
    }
}
