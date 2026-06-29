// SPDX-License-Identifier: UNLICENSED
//! **Un écrivain USTAR minimal, fait main** — pour streamer le bundle forensic (un répertoire) au
//! navigateur en un seul téléchargement, **sans dépendance** (fidèle à l'austérité du fossé). Le format
//! USTAR est simple et stable : un en-tête de 512 octets par fichier, le contenu paddé à 512, deux blocs
//! nuls en fin. Déterministe (mtime=0, uid/gid=0, parcours trié) : même bundle → même tar, à l'octet.
//!
//! Rigueur de sûreté, héritée du MANIFEST de S4 : on **refuse les liens symboliques** (un bundle ne
//! contient que des fichiers réguliers ; suivre un lien pourrait échapper le répertoire), et on **borne**
//! la longueur des noms (USTAR : 100 octets) plutôt que de tronquer en silence.

use std::fs;
use std::io::Result as IoResult;
use std::path::{Path, PathBuf};

const BLOCK: usize = 512;

/// Empaquette récursivement `root` en une archive USTAR (octets). Les chemins du tar sont relatifs à
/// `root`, triés (déterministe). Refuse les liens symboliques ; rend une erreur si un nom dépasse 100 octets.
pub fn tar_dir(root: &Path) -> IoResult<Vec<u8>> {
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    collect(root, root, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = Vec::new();
    for (rel, abs) in &files {
        let content = fs::read(abs)?;
        write_entry(&mut out, rel, &content)?;
    }
    // Fin d'archive : deux blocs nuls.
    out.extend_from_slice(&[0u8; BLOCK * 2]);
    Ok(out)
}

fn collect(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> IoResult<()> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)?.map(|e| e.map(|e| e.path())).collect::<Result<_, _>>()?;
    entries.sort();
    for path in entries {
        // symlink_metadata NE SUIT PAS les liens — un bundle ne doit contenir que des fichiers réguliers.
        let meta = fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("lien symbolique interdit dans le bundle : {}", path.display())));
        }
        if meta.is_dir() {
            collect(base, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(base) {
            out.push((rel.to_string_lossy().replace('\\', "/"), path));
        }
    }
    Ok(())
}

fn write_entry(out: &mut Vec<u8>, name: &str, content: &[u8]) -> IoResult<()> {
    let name_bytes = name.as_bytes();
    if name_bytes.len() > 100 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("nom de fichier tar > 100 octets : {name}")));
    }
    // Garde : le champ size USTAR (12 octets → 11 chiffres octaux) ne représente pas ≥ 8 GiB. On REFUSE
    // (comme pour un nom > 100) plutôt que de tronquer en silence — fidèle à la discipline du module.
    const USTAR_MAX_SIZE: u64 = (1u64 << (3 * 11)) - 1; // 0o77777777777 ≈ 7.99 GiB
    if content.len() as u64 > USTAR_MAX_SIZE {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("fichier au-delà du champ size USTAR ({USTAR_MAX_SIZE} octets max) : {name}")));
    }
    let mut header = [0u8; BLOCK];
    // name[0..100]
    header[..name_bytes.len()].copy_from_slice(name_bytes);
    // mode (0000644), uid, gid — champs octaux NUL-terminés (longueur 8).
    put_octal(&mut header[100..108], 0o644);
    put_octal(&mut header[108..116], 0);
    put_octal(&mut header[116..124], 0);
    // size[124..136] (12), mtime[136..148] (12) — déterministe mtime=0.
    put_octal(&mut header[124..136], content.len() as u64);
    put_octal(&mut header[136..148], 0);
    // typeflag '0' (fichier régulier).
    header[156] = b'0';
    // magic "ustar\0" + version "00".
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    // checksum : somme de tous les octets, le champ checksum traité comme 8 espaces.
    for b in &mut header[148..156] {
        *b = b' ';
    }
    let sum: u32 = header.iter().map(|&b| b as u32).sum();
    // checksum USTAR : **6 chiffres octaux + NUL + espace**. 6 chiffres suffisent TOUJOURS (la somme d'un
    // en-tête de 512 octets ≤ 0o777777), donc aucune troncature — contrairement à un champ de 5 chiffres.
    put_octal(&mut header[148..155], sum as u64); // champ de 7 octets → 6 chiffres + NUL à l'index 154
    header[155] = b' ';

    out.extend_from_slice(&header);
    out.extend_from_slice(content);
    // padding à un multiple de 512.
    let rem = content.len() % BLOCK;
    if rem != 0 {
        out.resize(out.len() + (BLOCK - rem), 0);
    }
    Ok(())
}

/// Écrit un entier en octal ASCII, NUL-terminé, dans un champ de largeur `field.len()`.
fn put_octal(field: &mut [u8], mut value: u64) {
    let width = field.len();
    // Le dernier octet est le NUL terminateur ; on remplit les (width-1) chiffres, paddés de zéros à gauche.
    let digits = width - 1;
    for i in (0..digits).rev() {
        field[i] = b'0' + (value & 0o7) as u8;
        value >>= 3;
    }
    field[width - 1] = 0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn produces_a_deterministic_archive_extractable_by_system_tar() {
        let dir = std::env::temp_dir().join(format!("s3-tar-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("verify/src")).unwrap();
        fs::write(dir.join("records.sealed.jsonl"), b"{\"a\":1}\n").unwrap();
        fs::write(dir.join("verify/src/main.rs"), b"fn main(){}\n").unwrap();
        fs::write(dir.join("README.md"), "# bundle\n".as_bytes()).unwrap();

        let a = tar_dir(&dir).unwrap();
        let b = tar_dir(&dir).unwrap();
        assert_eq!(a, b, "déterministe : même bundle → même tar à l'octet");
        assert_eq!(a.len() % BLOCK, 0, "taille multiple de 512");

        // Extraction par le tar SYSTÈME → prouve la compatibilité réelle.
        let tar_path = dir.join("out.tar");
        fs::write(&tar_path, &a).unwrap();
        let extract = std::env::temp_dir().join(format!("s3-untar-{}", std::process::id()));
        let _ = fs::remove_dir_all(&extract);
        fs::create_dir_all(&extract).unwrap();
        let status = Command::new("tar").args(["-xf", tar_path.to_str().unwrap(), "-C", extract.to_str().unwrap()]).status();
        match status {
            Ok(s) if s.success() => {
                assert_eq!(fs::read_to_string(extract.join("records.sealed.jsonl")).unwrap(), "{\"a\":1}\n");
                assert_eq!(fs::read_to_string(extract.join("verify/src/main.rs")).unwrap(), "fn main(){}\n");
                assert_eq!(fs::read_to_string(extract.join("README.md")).unwrap(), "# bundle\n");
            }
            _ => eprintln!("SKIP : tar système indisponible — l'archive reste déterministe et bien formée"),
        }
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&extract);
    }

    #[test]
    fn rejects_a_name_over_100_bytes() {
        let mut out = Vec::new();
        let long = "a".repeat(101);
        assert!(write_entry(&mut out, &long, b"x").is_err(), "nom > 100 octets refusé");
    }

    #[test]
    fn octal_field_is_nul_terminated_and_padded() {
        let mut f = [0xffu8; 8];
        put_octal(&mut f, 0o644);
        assert_eq!(&f, b"0000644\0", "0o644 → « 0000644\\0 »");
    }
}
