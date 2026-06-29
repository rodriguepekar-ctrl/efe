// SPDX-License-Identifier: UNLICENSED
//! **Le README du sceptique** — « comment vérifier VOUS-MÊME, sans nous faire confiance ». Il **porte**
//! l'honnêteté légalement nécessaire : ce que la re-preuve prouve, et ce qu'elle ne prouve **pas**
//! (intègre ≠ correct ; et la distinction neuve de S4 : **intègre ≠ inforgeable** → l'ancrage). C'est le
//! support de l'invariant `I-S4-INTÈGRE≠INFORGEABLE` (test de présence du libellé).

/// Le contexte de rendu du README (groupé pour une signature nette).
#[derive(Debug, Clone)]
pub struct ReadmeCtx<'a> {
    pub tenant_label: &'a str,
    pub manifest_root: &'a str,
    pub n_records: usize,
    pub sealed_scope: &'a str,
    pub report_cases: &'a [String],
    pub vendored_lines: usize,
    pub has_anchor: bool,
}

/// Rend le README.md du bundle.
pub fn render(ctx: &ReadmeCtx) -> String {
    let ReadmeCtx { tenant_label, manifest_root, n_records, sealed_scope, report_cases, vendored_lines, has_anchor } = *ctx;
    let anchor_block = if has_anchor {
        "Ce bundle inclut **`anchor.proof`** : une preuve d'**ancrage externe** du `manifest.root` (p. ex. \
un jeton d'horodatage RFC 3161, ou un reçu de log de transparence). Elle atteste que **ce root existait \
au temps T** — donc que le record a été **engagé avant tout litige**, et n'a pas pu être fabriqué \
a posteriori. Vérifiez `anchor.proof` auprès de l'autorité d'ancrage indiquée, **indépendamment** de nous."
            .to_string()
    } else {
        "Ce bundle **n'inclut pas** de preuve d'ancrage externe. La re-preuve atteste alors l'intégrité et \
la **non-répudiation par le détenteur de la clé** — mais **pas** que ce détenteur n'a pas **fabriqué** le \
record (voir « Ce que ça NE prouve PAS »). Pour une preuve opposable au détenteur lui-même, exigez un \
**ancrage** du root (RFC 3161 / log de transparence)."
            .to_string()
    };

    let cases_line = if report_cases.is_empty() {
        String::new()
    } else {
        format!("\n- **Dossiers du récit** : {}\n", report_cases.join(", "))
    };

    format!(
        "# Bundle forensic — tenant « {tenant_label} »\n\
\n\
Un **artefact autonome** : ces records sont scellés et **re-prouvables sans nous faire confiance**. \
Vous ({tenant_label}) ou votre auditeur/régulateur pouvez vérifier leur **intégrité** hors-ligne, en \
**re-compilant** le vérifieur fourni — sans rien installer de Kovant ni d'EFE, et même si Kovant n'existe \
plus dans dix ans.\n\
\n\
- **manifest-root** : `{manifest_root}`\n\
- **records** : {n_records} (`records.sealed.jsonl`)\n\
- **portée scellée** : {sealed_scope}{cases_line}\
\n\
## Comment vérifier VOUS-MÊME (sans nous faire confiance)\n\
\n\
1. **Lisez** la source du vérifieur dans `verify/` (~{vendored_lines} lignes : SHA-256 + HMAC écrits à la \
main, prouvés par vecteurs NIST/RFC, et le CLI). Lisible en une heure.\n\
2. **Re-compilez-la** : `cd verify && ./build.sh` — vous obtenez **VOTRE** binaire, pas le nôtre (un \
binaire livré pourrait mentir ; le vôtre ne le peut pas).\n\
3. **Obtenez la clé tenant** `K_tenant` auprès du **responsable de traitement** (elle n'est **PAS** dans \
ce bundle — voir « La clé »).\n\
4. **Vérifiez** :\n\
   ```\n\
   ./verify/provenance-verify/target/release/provenance-verify \\\n\
       records.sealed.jsonl --key-hex <K_tenant> --root {manifest_root}\n\
   ```\n\
   Code de sortie **0** ⇔ « ces records sont **authentiques &amp; non-altérés**, scellés sous K_tenant ».\n\
\n\
## Ce que ça PROUVE\n\
\n\
- **Intégrité** : le record est authentique et **non-altéré** (toute mutation casse la chaîne de sceaux).\n\
- **Non-répudiation par le détenteur de la clé** : il a été scellé sous `K_tenant`.\n\
\n\
## Ce que ça NE prouve PAS (honnêteté légalement nécessaire)\n\
\n\
- **≠ correct** : la re-preuve atteste *ce qui s'est passé*, **pas** la **justesse** de la décision (un \
modèle peut se tromper proprement). La justesse est une autre couche.\n\
- **≠ reproductible** bit-à-bit (les modèles sont non-déterministes).\n\
- **≠ inforgeable** par le détenteur de la clé : quiconque détient `K_tenant` peut sceller une chaîne \
cohérente sur un contenu **arbitraire**. La re-preuve seule n'exclut donc **pas** une fabrication par le \
détenteur lui-même. {anchor_block}\n\
\n\
## La clé voyage SÉPARÉMENT\n\
\n\
Ce bundle **ne contient pas** `K_tenant` (souveraineté : la clé reste dans la frontière du client). \
Obtenez-la auprès du responsable de traitement, par votre canal de confiance. Sans la clé, ce bundle \
**ne peut pas être re-scellé** de façon cohérente (forger exige la clé) ; il est donc tamper-evident en \
transit. Une clé d'un **autre** tenant échoue dès le premier sceau.\n\
\n\
## Le récit est une VUE, pas la preuve\n\
\n\
`report.md` est une **commodité** de lecture — **chaque ligne cite le `node_id`** scellé dont elle dérive. \
La garantie **que ce bundle vous donne à lui seul** : ce node_id **existe** dans `records.sealed.jsonl` \
(re-prouvés) et son `output` scellé **correspond** à la ligne — vous le **recoupez** directement, sans rien \
d'autre. L'**évidence**, ce sont les records + le vérifieur ; **jamais** le texte du récit. Pour une \
**régénération exacte** (diff au caractère près), il faut le **rendeur de projection** (le composant S2), \
obtenu **séparément** — il n'est **pas** embarqué ici, et le bundle ne le réclame pas : sa garantie propre \
est la **traçabilité par node_id** ci-dessus. Ne faites jamais confiance au texte livré : recoupez-le.\n\
\n\
## MANIFEST &amp; longévité\n\
\n\
`MANIFEST.sha256` liste le `sha256` de chaque fichier — une intégrité de **transport** (détecter une \
corruption), **pas** une frontière de sécurité (il n'est pas signé ; la sécurité vient de la re-preuve). \
Tout est ici : données, vérifieur (source), spéc, guide — rien à télécharger, personne à appeler. \
Le bundle se vérifie **hors-ligne**, et **survit au vendeur**.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(root: &'a str, cases: &'a [String], anchor: bool) -> ReadmeCtx<'a> {
        ReadmeCtx { tenant_label: "acme", manifest_root: root, n_records: 6, sealed_scope: "full-tenant-dag", report_cases: cases, vendored_lines: 400, has_anchor: anchor }
    }

    #[test]
    fn readme_carries_the_integrite_neq_inforgeable_statement() {
        let root = "ab".repeat(32);
        let cases = vec!["SUP-A".to_string()];
        let md = render(&ctx(&root, &cases, false));
        // I-S4-INTÈGRE≠INFORGEABLE : le libellé est présent (les deux distinctions).
        assert!(md.contains("≠ correct"), "intègre ≠ correct énoncé");
        assert!(md.contains("≠ inforgeable"), "intègre ≠ inforgeable énoncé");
        assert!(md.to_lowercase().contains("ancrage"), "l'ancrage est expliqué");
        assert!(md.contains("ne contient pas"), "la clé voyage séparément");
        assert!(md.contains("VUE, pas la preuve"), "le récit est une vue");
        assert!(md.contains(&"ab".repeat(32)), "le root est cité");
    }

    #[test]
    fn anchor_block_changes_with_presence() {
        let none: Vec<String> = vec![];
        let with = render(&ctx("r", &none, true));
        let without = render(&ctx("r", &none, false));
        assert!(with.contains("anchor.proof"), "bloc ancrage présent");
        assert!(without.contains("n'inclut pas"), "bloc sans-ancrage honnête");
        assert_ne!(with, without);
    }
}
