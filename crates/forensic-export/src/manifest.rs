// SPDX-License-Identifier: UNLICENSED
//! **Le MANIFEST de hachages** — `sha256` (du cœur, écrit à la main) de **chaque** fichier du bundle.
//! C'est une **intégrité de TRANSPORT** (détecter une corruption/un fichier échangé), **pas** une frontière
//! de sécurité : la sécurité vient de la re-preuve des records scellés (le MANIFEST n'est pas signé — il
//! n'y a pas de clé dans le bundle, §7). Format `sha256sum` : `<hex>  <chemin-relatif>`.

use provenance_dag::sha256;
use std::fs;
use std::path::{Path, PathBuf};

/// `sha256` → hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let d = sha256(bytes);
    let mut s = String::with_capacity(64);
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Liste **récursivement** les fichiers sous `root` (chemins relatifs à `root`), triés — déterministe.
/// Exclut un éventuel `MANIFEST.sha256` (on ne se hache pas soi-même).
pub fn list_files(root: &Path) -> std::io::Result<Vec<String>> {
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    out.retain(|p| p != "MANIFEST.sha256");
    out.sort();
    Ok(out)
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)?.map(|e| e.map(|e| e.path())).collect::<Result<_, _>>()?;
    entries.sort();
    for path in entries {
        // `symlink_metadata` ne SUIT PAS les liens : un bundle forensic ne doit contenir que des fichiers
        // réguliers (suivre un lien planté pourrait échapper le dossier ou rendre le hachage non-déterministe).
        let meta = fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("lien symbolique interdit dans le bundle : {}", path.display()),
            ));
        }
        if meta.is_dir() {
            walk(base, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(base) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

/// Calcule le MANIFEST sur tous les fichiers de `root` et l'écrit en `root/MANIFEST.sha256`. Rend la
/// liste `(chemin, hex)` (déterministe, triée).
pub fn write_manifest(root: &Path) -> std::io::Result<Vec<(String, String)>> {
    let files = list_files(root)?;
    let mut rows = Vec::with_capacity(files.len());
    let mut body = String::new();
    for rel in &files {
        let bytes = fs::read(root.join(rel))?;
        let hex = sha256_hex(&bytes);
        body.push_str(&format!("{hex}  {rel}\n"));
        rows.push((rel.clone(), hex));
    }
    fs::write(root.join("MANIFEST.sha256"), body)?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_known_empty() {
        // SHA-256("") connu — le hachage est bien celui du cœur.
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn manifest_covers_all_files_recursively_and_is_deterministic() {
        let dir = std::env::temp_dir().join(format!("s4-manifest-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("a.txt"), b"alpha").unwrap();
        fs::write(dir.join("sub/b.txt"), b"beta").unwrap();
        let rows = write_manifest(&dir).unwrap();
        // récursif : a.txt + sub/b.txt (et PAS MANIFEST lui-même).
        let names: Vec<&str> = rows.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(names, vec!["a.txt", "sub/b.txt"], "tous les fichiers, triés, sans MANIFEST");
        // le hash de a.txt est bien sha256("alpha").
        assert_eq!(rows[0].1, sha256_hex(b"alpha"));
        // déterministe : ré-écrire donne le même contenu.
        let again = std::fs::read_to_string(dir.join("MANIFEST.sha256")).unwrap();
        write_manifest(&dir).unwrap();
        assert_eq!(again, std::fs::read_to_string(dir.join("MANIFEST.sha256")).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
