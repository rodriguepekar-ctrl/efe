// SPDX-License-Identifier: UNLICENSED
//! # `verify_meta` — re-preuve AUTONOME d'un consensus de méta-essaim (2 fichiers + 1 clé).
//!
//! Un tiers sceptique re-compile ce binaire et re-prouve, **hors-ligne**, qu'une suite de
//! pas de consensus de méta-essaim est authentique et non-altérée — **sans nous**. Il ne
//! fait confiance qu'aux maths (le sceau HMAC/BLAKE3, byte-identique à nano/essaim) et à
//! sa propre compilation. La **clé tenant** vient du responsable de traitement, jamais des
//! fichiers.
//!
//! Usage : `verify_meta <meta_steps.jsonl> <meta_manifest.json> --key-hex <64hex> [--json]`
//! Codes de sortie : `0` intègre · `1` altéré/réordonné/mauvaise clé (localisé) · `2` usage/E-S.

use efe_meta_verify::{check_binding, from_hex, verify, MetaManifest, SealedMetaStep, Verdict};
use std::process::exit;

const USAGE: &str = "usage : verify_meta <meta_steps.jsonl> <meta_manifest.json> --key-hex <64hex> [--json]";

fn main() {
    exit(real_main(&std::env::args().collect::<Vec<_>>()));
}

/// Point d'entrée testable : args → code de sortie. N'imprime que des verdicts/erreurs.
fn real_main(args: &[String]) -> i32 {
    let (mut steps_path, mut manifest_path, mut key_hex, mut json) = (None, None, None, false);
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--key-hex" => {
                i += 1;
                key_hex = args.get(i).cloned();
            }
            "--json" => json = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return 0;
            }
            other if steps_path.is_none() => steps_path = Some(other.to_string()),
            other if manifest_path.is_none() => manifest_path = Some(other.to_string()),
            other => {
                eprintln!("argument inattendu : {other}\n\n{USAGE}");
                return 2;
            }
        }
        i += 1;
    }

    let (steps_path, manifest_path, key_hex) = match (steps_path, manifest_path, key_hex) {
        (Some(a), Some(b), Some(k)) => (a, b, k),
        _ => {
            eprintln!("erreur : arguments manquants\n\n{USAGE}");
            return 2;
        }
    };

    let key = match from_hex(&key_hex) {
        Some(k) if !k.is_empty() => k,
        _ => {
            eprintln!("erreur : --key-hex invalide (chaîne hex non vide attendue)");
            return 2;
        }
    };

    let steps = match read_steps(&steps_path) {
        Ok(s) => s,
        Err(code) => return code,
    };

    let manifest_raw = match std::fs::read_to_string(&manifest_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("erreur : impossible de lire « {manifest_path} » : {e}");
            return 2;
        }
    };
    let manifest: MetaManifest = match serde_json::from_str(&manifest_raw) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("erreur : manifeste illisible : {e}");
            return 2;
        }
    };

    // Garde de forme : le manifeste doit décrire exactement ces pas.
    if manifest.n_steps != steps.len() {
        eprintln!("erreur : manifeste incohérent (n_steps={} ≠ {} lignes)", manifest.n_steps, steps.len());
        return 2;
    }

    // **v4** — recouper la LIAISON (qui / sous quelle config / sous quelle règle) AVANT la chaîne.
    //
    // `verify` ne lit JAMAIS la liaison du manifeste : il ne regarde que le condensé scellé dans
    // chaque pas, auquel un adversaire sans la clé tenant ne peut pas toucher. Une identité
    // réécrite ICI laisse donc la chaîne parfaitement intègre — et sans ce contrôle, ce binaire
    // imprimait « ✓ VÉRIFIÉ — authentique et non-altéré », code 0, sur un manifeste aux noms
    // permutés, pendant que `verify_meta_mini` le refusait. Les deux vérifieurs livrés doivent
    // rendre le MÊME verdict, sans quoi « re-prouvable sans nous faire confiance » ne veut rien
    // dire : le tiers choisirait lequel croire.
    //
    // AVANT la chaîne, comme le mini : sinon une mauvaise clé masquerait une falsification
    // délibérée, et l'auditeur ne la découvrirait qu'après avoir corrigé sa clé.
    if let Err(e) = check_binding(&steps, manifest.binding().as_ref()) {
        // `root` reste vide : la chaîne n'a PAS été re-calculée, et prétendre le contraire dans le
        // JSON serait un petit mensonge de plus dans un outil dont c'est tout le propos.
        let verdict = Verdict { ok: false, root: String::new(), broken_at: None, reason: Some(e) };
        println!("{}", render(&verdict, &manifest, json, FAUTE_LIAISON));
        return 1;
    }

    let verdict = verify(&steps, &key, &manifest.root);
    println!("{}", render(&verdict, &manifest, json, FAUTE_CHAINE));
    i32::from(!verdict.ok) // 0 intègre, 1 échec
}

/// Les deux fautes ne se ressemblent pas, et l'auditeur n'agit pas pareil selon celle qu'il lit.
///
/// Sur une chaîne cassée, il soupçonne le transfert, le fichier, sa clé. Sur une liaison refusée,
/// il sait que le contenu scellé est intact et que quelqu'un a réécrit les NOMS — une altération
/// délibérée, pas une corruption. Le titre d'échec historique racontait la première histoire pour
/// les deux ; sur une liaison falsifiée, « contenu altéré » est faux.
const FAUTE_CHAINE: &str = "re-preuve refusée (contenu altéré, réordonné, mauvaise clé, ou manifeste incohérent).";
/// Le titre d'un refus de liaison — cf. [`FAUTE_CHAINE`].
const FAUTE_LIAISON: &str = "liaison refusée — la CHAÎNE est intègre, mais le manifeste ne nomme PAS les acteurs qui ont été scellés.";

/// Lit et parse le `meta_steps.jsonl` (une [`SealedMetaStep`] par ligne non vide).
fn read_steps(path: &str) -> Result<Vec<SealedMetaStep>, i32> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        eprintln!("erreur : impossible de lire « {path} » : {e}");
        2
    })?;
    let mut out = Vec::new();
    for (n, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(s) => out.push(s),
            Err(e) => {
                eprintln!("erreur : ligne JSONL {n} illisible : {e}");
                return Err(2);
            }
        }
    }
    Ok(out)
}

/// Rend le verdict — la **forme des octets**, séparée de leur impression pour être testable.
///
/// Un refus qui ne dit rien sur `--json` (code 1, stdout vide) laisse un consommateur machine
/// devant « refusé, motif inconnu ». C'est ce que faisait le chemin de liaison avant d'exister,
/// et c'est ce qu'aurait fait un simple `eprintln!` copié du mini.
///
/// `titre` est ignoré quand le verdict est bon : il n'y a qu'une façon de réussir.
fn render(v: &Verdict, m: &MetaManifest, json: bool, titre: &str) -> String {
    if json {
        return verdict_json(v, m);
    }
    if v.ok {
        return format!(
            "✓ VÉRIFIÉ — consensus méta-essaim « {} » authentique et non-altéré ({} pas, root concordant).\n  meta_provenance_root : {}\n  (intégrité attestée — PAS la correction de la décision de consensus)",
            m.meta_id, m.n_steps, v.root
        );
    }
    let mut s = format!("✗ ÉCHEC — {titre}");
    if let Some(k) = v.broken_at {
        s.push_str(&format!("\n  cassé au pas : {k}"));
    }
    if let Some(r) = &v.reason {
        s.push_str(&format!("\n  motif : {r}"));
    }
    s
}

fn verdict_json(v: &Verdict, m: &MetaManifest) -> String {
    let broken = v.broken_at.map_or_else(|| "null".to_string(), |x| x.to_string());
    let reason = v.reason.as_ref().map_or_else(|| "null".to_string(), |r| format!("{r:?}"));
    format!(
        "{{\"ok\":{},\"meta_id\":{:?},\"n_steps\":{},\"root\":{:?},\"broken_at\":{broken},\"reason\":{reason}}}",
        v.ok, m.meta_id, m.n_steps, v.root
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_meta_verify::{MetaChainState, RunBinding};
    use std::path::{Path, PathBuf};

    /// La liaison d'un run de test — un participant qui déclare une configuration, un mock qui n'en
    /// a aucune à attester : les deux états dans la même liaison.
    fn liaison() -> RunBinding {
        RunBinding::try_new(vec!["supplier-scoring".into(), "contract-risk".into()], vec!["a".repeat(64), String::new()], "uniform", 0.5, 0.2)
            .expect("liaison bien formée")
    }

    /// Écrit un bundle scellé dans un répertoire temporaire, et rend `(répertoire, clé en hex)`.
    ///
    /// Sans `tempfile` : le `cargo tree` court de ce vérifieur est un argument de vente auprès du
    /// tiers sceptique qui le recompile, on ne l'allonge pas pour un test.
    fn bundle(nom: &str, binding: Option<RunBinding>) -> (PathBuf, String) {
        let key = [0x9cu8; 32];
        let mut chain = match binding {
            Some(b) => MetaChainState::new_bound("m", key.to_vec(), b),
            None => MetaChainState::new("m", key.to_vec()),
        };
        for i in 0..3u64 {
            chain.push(i, 1.0 / (f64::from(i as u32) + 1.0), vec![0.1, 0.2], Some(0.0), Some(vec!["b".repeat(64)]));
        }
        let dir = std::env::temp_dir().join(format!("verify_meta_{}_{nom}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        chain.finish().write_bundle(&dir).expect("bundle écrit");
        (dir, key.iter().map(|b| format!("{b:02x}")).collect())
    }

    fn args(dir: &Path, manifeste: &str, key_hex: &str) -> Vec<String> {
        vec![
            "verify_meta".into(),
            dir.join("meta_steps.jsonl").display().to_string(),
            dir.join(manifeste).display().to_string(),
            "--key-hex".into(),
            key_hex.into(),
        ]
    }

    /// **LA régression.** Un manifeste dont on a PERMUTÉ deux identités — pas scellés intouchés à
    /// l'octet près — était accepté « authentique et non-altéré », code 0, pendant que
    /// `verify_meta_mini` refusait le même fichier. Deux vérifieurs livrés qui rendent des verdicts
    /// opposés sur un même bundle ruinent la promesse : le tiers devrait choisir lequel croire.
    #[test]
    fn a_permuted_identity_in_the_manifest_is_refused() {
        let (dir, key) = bundle("permute", Some(liaison()));
        assert_eq!(real_main(&args(&dir, "meta_manifest.json", &key)), 0, "le bundle honnête est accepté");

        // L'adversaire n'a pas la clé tenant : il ne peut toucher qu'au fichier qui n'est PAS scellé.
        let honnete = std::fs::read_to_string(dir.join("meta_manifest.json")).expect("manifeste lisible");
        let mut m: MetaManifest = serde_json::from_str(&honnete).expect("manifeste relisible");
        m.binding_participant_ids.as_mut().expect("v4 ⟹ liaison").swap(0, 1);
        m.binding_declared_config_hashes.as_mut().expect("v4 ⟹ liaison").swap(0, 1);
        std::fs::write(dir.join("FORGE.json"), serde_json::to_string_pretty(&m).expect("sérialisable")).expect("écrit");

        assert_eq!(real_main(&args(&dir, "FORGE.json", &key)), 1, "identités permutées ⟹ REFUS");
    }

    /// **Rétro-compatibilité.** Un bundle v1-v3 n'a aucune liaison à recouper, et le contrôle ne
    /// doit pas le refuser — sinon v4 invaliderait rétroactivement tous les reçus déjà émis.
    #[test]
    fn a_v3_bundle_without_a_binding_is_still_accepted() {
        let (dir, key) = bundle("v3", None);
        assert_eq!(real_main(&args(&dir, "meta_manifest.json", &key)), 0, "un bundle v3 reste accepté");
    }

    /// Le chemin de SUCCÈS rend **exactement** les octets d'avant l'extraction de `print_human`
    /// vers [`render`] — trois lignes, deux espaces d'indentation. Ce n'est pas de la cosmétique :
    /// c'est la seule sortie que lit un auditeur quand tout va bien, et la réécriture l'a abîmée
    /// une fois (quinze espaces au lieu de deux) sans qu'aucun test ne s'en aperçoive.
    #[test]
    fn the_success_output_is_byte_for_byte_what_it_was() {
        let m = MetaManifest {
            meta_id: "m".into(),
            schema_version: 4,
            n_steps: 3,
            root: "ab".repeat(32),
            binding_participant_ids: None,
            binding_declared_config_hashes: None,
            binding_operator: None,
            binding_alpha: None,
            binding_gamma: None,
        };
        let v = Verdict { ok: true, root: "ab".repeat(32), broken_at: None, reason: None };
        assert_eq!(
            render(&v, &m, false, FAUTE_CHAINE),
            format!(
                "\u{2713} VÉRIFIÉ — consensus méta-essaim « m » authentique et non-altéré (3 pas, root concordant).\n  meta_provenance_root : {}\n  (intégrité attestée — PAS la correction de la décision de consensus)",
                "ab".repeat(32)
            )
        );
    }

    /// Le `--json` ne doit pas être MUET sur un refus de liaison. Un consommateur machine qui reçoit
    /// « code 1, stdout vide » ne sait pas ce qu'on lui refuse — c'est ce qu'aurait produit un
    /// `eprintln!` recopié du mini, dont le chemin de liaison n'écrit rien sur stdout.
    #[test]
    fn the_json_refusal_carries_the_binding_reason() {
        let m = MetaManifest {
            meta_id: "m".into(),
            schema_version: 4,
            n_steps: 3,
            root: "00".repeat(32),
            binding_participant_ids: None,
            binding_declared_config_hashes: None,
            binding_operator: None,
            binding_alpha: None,
            binding_gamma: None,
        };
        let v = Verdict { ok: false, root: String::new(), broken_at: None, reason: Some("pas 0 : … identité ou règle falsifiée".into()) };

        let json = render(&v, &m, true, FAUTE_LIAISON);
        assert!(json.contains("\"ok\":false"), "{json}");
        assert!(json.contains("falsifiée"), "le motif atteint le consommateur machine : {json}");

        // Et en humain, le titre ne raconte PAS l'histoire de la chaîne : le contenu est intact.
        let humain = render(&v, &m, false, FAUTE_LIAISON);
        assert!(humain.contains("la CHAÎNE est intègre"), "{humain}");
        assert!(!humain.contains("contenu altéré"), "un refus de liaison ne doit pas être annoncé comme une altération : {humain}");
    }
}
