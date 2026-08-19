// SPDX-License-Identifier: UNLICENSED
//! **Le parseur JSON fait main — ce qui remplace `serde`** (et rend l'arbre de dépendances vide).
//!
//! Surface étroite : exactement ce qu'un pas scellé contient — un **objet plat** dont les valeurs sont des
//! **chaînes**, des **nombres** (entier ou flottant) ou des **tableaux de nombres**. Deux points de rigueur :
//! (1) le **décodage exact** des chaînes (échappements `\" \\ \/ \n \r \t \b \f`, `\uXXXX` et paires de
//! substitution) ; (2) les nombres sont rendus en **token brut** puis interprétés par
//! `str::parse::<u64>()` / `str::parse::<f64>()` de la **std**, dont le parseur de flottants est
//! **correctement arrondi** (round-trip exact avec l'écriture ryu) — indispensable pour que le
//! méta-consensus se re-hache **à l'identique** que le sceau. On travaille sur des `char` (UTF-8 valide).

/// Une valeur JSON, restreinte à ce dont un pas a besoin.
#[derive(Debug, Clone)]
pub enum JsonValue {
    /// Chaîne (échappements décodés).
    Str(String),
    /// Nombre — token brut (entier ou flottant), interprété par l'appelant.
    Num(String),
    /// Tableau de nombres — tokens bruts.
    NumArr(Vec<String>),
    /// Tableau de chaînes (v3 : les `vote_hashes`).
    StrArr(Vec<String>),
}

/// Un objet JSON plat (recherche par **nom**, indépendante de l'ordre ; l'encodage canonique, lui, est figé).
#[derive(Debug, Clone)]
pub struct Obj {
    fields: Vec<(String, JsonValue)>,
}
impl Obj {
    /// La valeur chaîne d'un champ.
    pub fn get_str(&self, key: &str) -> Result<&str, String> {
        match self.lookup(key)? {
            JsonValue::Str(s) => Ok(s),
            other => Err(format!("champ « {key} » : chaîne attendue, trouvé {other:?}")),
        }
    }
    /// La valeur entière (u64) d'un champ.
    pub fn get_u64(&self, key: &str) -> Result<u64, String> {
        match self.lookup(key)? {
            JsonValue::Num(t) => t.parse::<u64>().map_err(|e| format!("champ « {key} » : entier invalide ({e})")),
            other => Err(format!("champ « {key} » : entier attendu, trouvé {other:?}")),
        }
    }
    /// La valeur flottante (f64) d'un champ — parse std, correctement arrondi.
    pub fn get_f64(&self, key: &str) -> Result<f64, String> {
        match self.lookup(key)? {
            JsonValue::Num(t) => t.parse::<f64>().map_err(|e| format!("champ « {key} » : flottant invalide ({e})")),
            other => Err(format!("champ « {key} » : flottant attendu, trouvé {other:?}")),
        }
    }
    /// La valeur tableau-de-flottants d'un champ.
    pub fn get_f64_arr(&self, key: &str) -> Result<Vec<f64>, String> {
        match self.lookup(key)? {
            JsonValue::NumArr(a) => a
                .iter()
                .map(|t| t.parse::<f64>().map_err(|e| format!("champ « {key} » : flottant invalide ({e})")))
                .collect(),
            other => Err(format!("champ « {key} » : tableau de flottants attendu, trouvé {other:?}")),
        }
    }
    /// La valeur flottante d'un champ **optionnel** : `None` si absent (bundle v1), `Some(f64)` si présent.
    pub fn get_f64_opt(&self, key: &str) -> Result<Option<f64>, String> {
        match self.fields.iter().find(|(k, _)| k == key) {
            None => Ok(None),
            Some((_, JsonValue::Num(t))) => t.parse::<f64>().map(Some).map_err(|e| format!("champ « {key} » : flottant invalide ({e})")),
            Some((_, other)) => Err(format!("champ « {key} » : flottant attendu, trouvé {other:?}")),
        }
    }
    /// La valeur tableau-de-chaînes d'un champ **optionnel** : `None` si absent (v1/v2), `Some(vec)` sinon.
    pub fn get_str_arr_opt(&self, key: &str) -> Result<Option<Vec<String>>, String> {
        match self.fields.iter().find(|(k, _)| k == key) {
            None => Ok(None),
            Some((_, JsonValue::StrArr(a))) => Ok(Some(a.clone())),
            Some((_, other)) => Err(format!("champ « {key} » : tableau de chaînes attendu, trouvé {other:?}")),
        }
    }
    /// **v4** — la valeur chaîne d'un champ **optionnel** : `None` si absent (v1/v2/v3).
    pub fn get_str_opt(&self, key: &str) -> Result<Option<String>, String> {
        match self.fields.iter().find(|(k, _)| k == key) {
            None => Ok(None),
            Some((_, JsonValue::Str(v))) => Ok(Some(v.clone())),
            Some((_, other)) => Err(format!("champ « {key} » : chaîne attendue, trouvé {other:?}")),
        }
    }
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
            Some('[') => self.array(),
            Some(c) if c == '-' || c.is_ascii_digit() => Ok(JsonValue::Num(self.number()?)),
            other => Err(format!("valeur inattendue {other:?} (offset {}) — chaîne, nombre, ou tableau attendus", self.i)),
        }
    }

    /// Un tableau : distingue **chaînes** (v3 `vote_hashes`) vs **nombres** (`consensus`) en regardant
    /// le premier caractère non-blanc après `[` — sans consommer.
    fn array(&mut self) -> Result<JsonValue, String> {
        let mut j = self.i + 1; // après '['
        while matches!(self.c.get(j), Some(' ' | '\t' | '\n' | '\r')) {
            j += 1;
        }
        if self.c.get(j) == Some(&'"') {
            Ok(JsonValue::StrArr(self.str_array()?))
        } else {
            Ok(JsonValue::NumArr(self.num_array()?))
        }
    }

    fn str_array(&mut self) -> Result<Vec<String>, String> {
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

    /// Un token numérique JSON : `-? digits (.digits)? ([eE][+-]?digits)?`. Rendu **brut** ;
    /// l'interprétation (u64 / f64) est faite par l'appelant via la std (correctement arrondie).
    fn number(&mut self) -> Result<String, String> {
        let start = self.i;
        if self.peek() == Some('-') {
            self.i += 1;
        }
        let d0 = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.i += 1;
        }
        if self.i == d0 {
            return Err(format!("chiffres attendus dans le nombre (offset {})", self.i));
        }
        if self.peek() == Some('.') {
            self.i += 1;
            let f0 = self.i;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.i += 1;
            }
            if self.i == f0 {
                return Err("décimales attendues après le point".into());
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.i += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.i += 1;
            }
            let e0 = self.i;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.i += 1;
            }
            if self.i == e0 {
                return Err("exposant attendu".into());
            }
        }
        Ok(self.c[start..self.i].iter().collect())
    }

    fn num_array(&mut self) -> Result<Vec<String>, String> {
        self.expect('[')?;
        let mut out = Vec::new();
        self.ws();
        if self.peek() == Some(']') {
            self.bump();
            return Ok(out);
        }
        loop {
            self.ws();
            out.push(self.number()?);
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
    fn parses_a_meta_step() {
        let o = parse_object(r#"{"meta_id":"kv","step":3,"residual":7.49e-1,"consensus":[-0.19,0.32,1.0],"seal":"ab"}"#).unwrap();
        assert_eq!(o.get_str("meta_id").unwrap(), "kv");
        assert_eq!(o.get_u64("step").unwrap(), 3);
        assert!((o.get_f64("residual").unwrap() - 0.749).abs() < 1e-12);
        assert_eq!(o.get_f64_arr("consensus").unwrap(), vec![-0.19, 0.32, 1.0]);
        assert_eq!(o.get_str("seal").unwrap(), "ab");
    }

    #[test]
    fn negative_and_exponent_numbers() {
        let o = parse_object(r#"{"a":[-1.5e-10,3E2,-0.0,42]}"#).unwrap();
        let v = o.get_f64_arr("a").unwrap();
        assert!((v[0] - (-1.5e-10)).abs() < 1e-25);
        assert_eq!(v[1], 300.0);
        assert_eq!(v[3], 42.0);
    }

    #[test]
    fn raw_utf8_and_escapes() {
        let o = parse_object("{\"meta_id\":\"café — v1\",\"seal\":\"a\\\"b\"}").unwrap();
        assert_eq!(o.get_str("meta_id").unwrap(), "café — v1");
        assert_eq!(o.get_str("seal").unwrap(), "a\"b");
    }

    #[test]
    fn rejects_malformed() {
        assert!(parse_object(r#"{"a":1"#).is_err(), "objet non fermé");
        assert!(parse_object(r#"{"a":1} junk"#).is_err(), "résidu");
        assert!(parse_object(r#"{"a":1.}"#).is_err(), "décimales manquantes");
        assert!(parse_object(r#"{"a":"x"#).is_err(), "chaîne non terminée");
    }
}
