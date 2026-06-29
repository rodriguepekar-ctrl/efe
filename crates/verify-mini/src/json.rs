// SPDX-License-Identifier: UNLICENSED
//! **Le parseur JSONL fait main — ce qui remplace `serde`** (et rend l'arbre de dépendances vide).
//!
//! Surface étroite : exactement ce qu'un *record* scellé contient — un **objet plat** dont les valeurs sont
//! des **chaînes**, des **entiers** (`timestamp`), ou des **tableaux de chaînes** (`evidence_refs`). La
//! rigueur porte sur le **décodage exact** des chaînes (échappements `\" \\ \/ \n \r \t \b \f`, `\uXXXX`
//! et **paires de substitution**) : la chaîne reconstruite doit être **identique octet pour octet** à celle
//! que S0 a scellée, sinon l'encodage canonique — et donc le sceau — ne concorderait pas. C'est précisément
//! ce que le cross-check contre S0 (`tests/`) prouve. On travaille sur des **`char`** (l'entrée est de
//! l'UTF-8 valide), donc l'UTF-8 multi-octets passe naturellement.

/// Une valeur JSON, restreinte à ce dont un record a besoin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonValue {
    Str(String),
    Num(u64),
    Arr(Vec<String>),
}

/// Un objet JSON plat (l'ordre des champs est conservé ; la recherche se fait par **nom**, donc
/// indépendante de l'ordre — l'encodage canonique, lui, est en ordre figé).
#[derive(Debug, Clone)]
pub struct Obj {
    fields: Vec<(String, JsonValue)>,
}
impl Obj {
    /// La valeur chaîne d'un champ (erreur si absent ou d'un autre type).
    pub fn get_str(&self, key: &str) -> Result<&str, String> {
        match self.lookup(key)? {
            JsonValue::Str(s) => Ok(s),
            other => Err(format!("champ « {key} » : chaîne attendue, trouvé {other:?}")),
        }
    }
    /// La valeur entière (u64) d'un champ.
    pub fn get_u64(&self, key: &str) -> Result<u64, String> {
        match self.lookup(key)? {
            JsonValue::Num(n) => Ok(*n),
            other => Err(format!("champ « {key} » : entier attendu, trouvé {other:?}")),
        }
    }
    /// La valeur tableau-de-chaînes d'un champ.
    pub fn get_arr(&self, key: &str) -> Result<&[String], String> {
        match self.lookup(key)? {
            JsonValue::Arr(a) => Ok(a),
            other => Err(format!("champ « {key} » : tableau attendu, trouvé {other:?}")),
        }
    }
    /// La PREMIÈRE occurrence d'un champ (déterministe ; un doublon forgé serait de toute façon attrapé
    /// par le sceau, qui couvre les champs réels).
    fn lookup(&self, key: &str) -> Result<&JsonValue, String> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v).ok_or_else(|| format!("champ manquant : « {key} »"))
    }
}

/// Parse **une** ligne JSON (un objet plat). Rejette tout résidu non-blanc après l'objet.
pub fn parse_object(line: &str) -> Result<Obj, String> {
    let mut p = P { c: line.chars().collect(), i: 0 };
    let fields = p.object()?;
    p.ws();
    if p.peek().is_some() {
        return Err(format!("résidu après l'objet JSON (offset {})", p.i));
    }
    Ok(Obj { fields })
}

/// Le curseur de parsing (sur des `char` — l'entrée est de l'UTF-8 valide).
struct P {
    c: Vec<char>,
    i: usize,
}
impl P {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }
    fn bump(&mut self) -> Option<char> {
        let x = self.peek();
        if x.is_some() {
            self.i += 1;
        }
        x
    }
    fn ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.i += 1;
        }
    }
    fn expect(&mut self, want: char) -> Result<(), String> {
        self.ws();
        match self.bump() {
            Some(c) if c == want => Ok(()),
            other => Err(format!("attendu « {want} », trouvé {other:?} (offset {})", self.i)),
        }
    }

    fn object(&mut self) -> Result<Vec<(String, JsonValue)>, String> {
        self.expect('{')?;
        let mut out = Vec::new();
        self.ws();
        if self.peek() == Some('}') {
            self.bump();
            return Ok(out);
        }
        loop {
            self.ws();
            let key = self.string()?;
            self.expect(':')?;
            let val = self.value()?;
            out.push((key, val));
            self.ws();
            match self.bump() {
                Some(',') => continue,
                Some('}') => break,
                other => return Err(format!("attendu « , » ou « }} », trouvé {other:?} (offset {})", self.i)),
            }
        }
        Ok(out)
    }

    fn value(&mut self) -> Result<JsonValue, String> {
        self.ws();
        match self.peek() {
            Some('"') => Ok(JsonValue::Str(self.string()?)),
            Some('[') => Ok(JsonValue::Arr(self.string_array()?)),
            Some(c) if c.is_ascii_digit() => Ok(JsonValue::Num(self.number()?)),
            other => Err(format!("valeur inattendue {other:?} (offset {}) — chaîne, entier ≥ 0, ou tableau de chaînes attendus", self.i)),
        }
    }

    /// Une chaîne JSON, échappements décodés exactement.
    fn string(&mut self) -> Result<String, String> {
        self.ws();
        if self.bump() != Some('"') {
            return Err(format!("chaîne attendue (offset {})", self.i));
        }
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err("chaîne non terminée".into()),
                Some('"') => return Ok(out),
                Some('\\') => match self.bump() {
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some('/') => out.push('/'),
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some('t') => out.push('\t'),
                    Some('b') => out.push('\u{0008}'),
                    Some('f') => out.push('\u{000C}'),
                    Some('u') => out.push(self.unicode_escape()?),
                    other => return Err(format!("échappement invalide : \\{other:?}")),
                },
                // Un caractère de contrôle nu (non échappé) est interdit par JSON ; on le refuse.
                Some(c) if (c as u32) < 0x20 => return Err("caractère de contrôle nu dans une chaîne".into()),
                Some(c) => out.push(c),
            }
        }
    }

    /// `\uXXXX`, avec gestion des **paires de substitution** (haute D800–DBFF + basse DC00–DFFF).
    fn unicode_escape(&mut self) -> Result<char, String> {
        let cp = self.hex4()?;
        if (0xD800..=0xDBFF).contains(&cp) {
            if self.bump() != Some('\\') || self.bump() != Some('u') {
                return Err("paire de substitution incomplète".into());
            }
            let lo = self.hex4()?;
            if !(0xDC00..=0xDFFF).contains(&lo) {
                return Err("substitution basse invalide".into());
            }
            let scalar = 0x1_0000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
            char::from_u32(scalar).ok_or_else(|| "point de code invalide".into())
        } else if (0xDC00..=0xDFFF).contains(&cp) {
            Err("substitution basse isolée".into())
        } else {
            char::from_u32(cp).ok_or_else(|| "point de code invalide".into())
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let mut v = 0u32;
        for _ in 0..4 {
            let d = self.bump().and_then(|c| c.to_digit(16)).ok_or("échappement \\u invalide (4 chiffres hexa attendus)")?;
            v = v * 16 + d;
        }
        Ok(v)
    }

    /// Un entier non signé (le `timestamp` est un `u64` ≥ 0 ; on refuse le signe et le flottant).
    fn number(&mut self) -> Result<u64, String> {
        let start = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.i += 1;
        }
        if self.i == start {
            return Err("entier attendu".into());
        }
        // Pas de point décimal / exposant : nos entiers sont entiers.
        if matches!(self.peek(), Some('.' | 'e' | 'E')) {
            return Err("entier attendu (pas de flottant)".into());
        }
        let s: String = self.c[start..self.i].iter().collect();
        s.parse::<u64>().map_err(|e| format!("entier invalide : {e}"))
    }

    fn string_array(&mut self) -> Result<Vec<String>, String> {
        self.expect('[')?;
        let mut out = Vec::new();
        self.ws();
        if self.peek() == Some(']') {
            self.bump();
            return Ok(out);
        }
        loop {
            self.ws();
            out.push(self.string()?);
            self.ws();
            match self.bump() {
                Some(',') => continue,
                Some(']') => break,
                other => return Err(format!("attendu « , » ou « ] », trouvé {other:?} (offset {})", self.i)),
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_flat_record_object() {
        let o = parse_object(r#"{"kind":"agent_act","node_id":"c::a","output":"PASS","evidence_refs":["e1","e2"],"timestamp":42,"seal":"ab"}"#).unwrap();
        assert_eq!(o.get_str("kind").unwrap(), "agent_act");
        assert_eq!(o.get_str("node_id").unwrap(), "c::a");
        assert_eq!(o.get_u64("timestamp").unwrap(), 42);
        assert_eq!(o.get_arr("evidence_refs").unwrap(), &["e1".to_string(), "e2".to_string()]);
        assert_eq!(o.get_arr("evidence_refs").unwrap().len(), 2);
        assert!(o.get_str("absent").is_err());
    }

    #[test]
    fn decodes_escapes_exactly() {
        // Guillemet, antislash, slash, contrôle (\n), et \u — doivent redonner les octets exacts.
        let o = parse_object(r#"{"s":"a\"b\\c\/d\ne\u00e9f"}"#).unwrap();
        assert_eq!(o.get_str("s").unwrap(), "a\"b\\c/d\ne\u{e9}f", "echappements decodes a l identique");
    }

    #[test]
    fn decodes_surrogate_pairs() {
        // 😀 = U+1F600 = 😀.
        let o = parse_object(r#"{"s":"x😀y"}"#).unwrap();
        assert_eq!(o.get_str("s").unwrap(), "x\u{1F600}y", "paire de substitution → un seul char");
    }

    #[test]
    fn raw_utf8_passes_through() {
        // L'UTF-8 brut (non échappé) est conservé tel quel (S0 l'écrit ainsi pour le non-ASCII).
        let o = parse_object("{\"s\":\"café — déjà 日本\"}").unwrap();
        assert_eq!(o.get_str("s").unwrap(), "café — déjà 日本");
    }

    #[test]
    fn empty_array_and_object() {
        let o = parse_object(r#"{"refs":[],"n":0}"#).unwrap();
        assert!(o.get_arr("refs").unwrap().is_empty());
        assert_eq!(o.get_u64("n").unwrap(), 0);
        assert!(parse_object("{}").unwrap().get_str("x").is_err());
    }

    #[test]
    fn rejects_malformed() {
        assert!(parse_object(r#"{"a":1"#).is_err(), "objet non fermé");
        assert!(parse_object(r#"{"a":1} junk"#).is_err(), "résidu après l'objet");
        assert!(parse_object(r#"{"a":-3}"#).is_err(), "entier négatif refusé");
        assert!(parse_object(r#"{"a":1.5}"#).is_err(), "flottant refusé");
        assert!(parse_object(r#"{"a":"x"#).is_err(), "chaîne non terminée");
        assert!(parse_object(r#"{"a":"\uZZZZ"}"#).is_err(), "\\u invalide");
        assert!(parse_object(r#"{"a":["x",2]}"#).is_err(), "tableau non-chaîne refusé");
    }
}
