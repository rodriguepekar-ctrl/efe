// SPDX-License-Identifier: UNLICENSED
//! **HTTP/1.1 minimal, fait main** — le parsing d'une requête (entrée NON fiable) et l'écriture d'une
//! réponse. Zéro dépendance : on ne tire ni hyper ni tokio, fidèle à l'austérité du fossé.
//!
//! Surface volontairement étroite (une console locale, pas un serveur public) : ligne de requête, en-têtes,
//! corps `Content-Length` (pas de `chunked`, pas de pipelining). La rigueur porte sur la **sûreté** :
//! bornes de taille (anti-DoS mémoire), aucun `panic` sur entrée malformée, décodage `%XX` strict, rejet
//! des octets de contrôle dans le chemin. Le parsing ne décide **rien** de métier — il rend une structure
//! que la coquille mappe vers une [`audit_api::Request`].

use std::io::{BufRead, Read};

/// Borne de la ligne de requête (méthode + cible + version).
pub const MAX_REQUEST_LINE: usize = 8 * 1024;
/// Borne du bloc d'en-têtes complet.
pub const MAX_HEADERS: usize = 64 * 1024;
/// Borne du corps (un POST d'action humaine est minuscule ; on cape large mais fini).
pub const MAX_BODY: usize = 8 * 1024 * 1024;

/// La méthode HTTP (seulement ce dont la console a besoin ; le reste est `Other`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Other(String),
}
impl HttpMethod {
    fn parse(s: &str) -> HttpMethod {
        match s {
            "GET" => HttpMethod::Get,
            "POST" => HttpMethod::Post,
            other => HttpMethod::Other(other.to_string()),
        }
    }
}

/// Une requête HTTP parsée — chemin et query **décodés**, en-têtes **minuscules**.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: HttpMethod,
    /// La cible brute (`/cases/X?fw=gdpr`) — utile au log ; le chemin/query décodés sont à part.
    pub raw_target: String,
    /// Le chemin décodé (`/cases/X`).
    pub path: String,
    /// Les paires de query, décodées (`+` → espace, `%XX` → octet).
    pub query: Vec<(String, String)>,
    /// Les en-têtes (noms en minuscules, pour une recherche insensible à la casse).
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Erreur de parsing — la coquille la rend en 400/413/501 selon le cas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    /// Ligne de requête malformée ou trop longue.
    BadRequestLine(String),
    /// En-têtes malformés ou trop volumineux.
    BadHeaders(String),
    /// Corps trop volumineux (au-delà de [`MAX_BODY`]).
    BodyTooLarge,
    /// Chemin contenant un octet interdit (contrôle, NUL) ou un `%XX` invalide.
    BadPath(String),
    /// E/S pendant la lecture.
    Io(String),
}
impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::BadRequestLine(m) => write!(f, "ligne de requête invalide : {m}"),
            HttpError::BadHeaders(m) => write!(f, "en-têtes invalides : {m}"),
            HttpError::BodyTooLarge => write!(f, "corps trop volumineux"),
            HttpError::BadPath(m) => write!(f, "chemin invalide : {m}"),
            HttpError::Io(m) => write!(f, "E/S : {m}"),
        }
    }
}
impl std::error::Error for HttpError {}

impl HttpRequest {
    /// Cherche un en-tête (insensible à la casse — les noms sont déjà en minuscules).
    pub fn header(&self, name: &str) -> Option<&str> {
        let n = name.to_ascii_lowercase();
        self.headers.iter().find(|(k, _)| *k == n).map(|(_, v)| v.as_str())
    }

    /// La valeur d'un paramètre de query (le premier si répété).
    pub fn query_get(&self, key: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// **Lit et parse** une requête depuis un flux (le serveur). Lit l'en-tête jusqu'à `\r\n\r\n` (borné),
    /// puis exactement `Content-Length` octets de corps (borné). Aucun `panic` sur entrée malformée.
    pub fn read_from<R: Read>(stream: R) -> Result<HttpRequest, HttpError> {
        let mut reader = std::io::BufReader::new(stream);
        // 1. accumuler l'en-tête jusqu'à la ligne vide, en bornant la taille.
        let mut head = Vec::new();
        loop {
            let mut line = Vec::new();
            let n = read_line_capped(&mut reader, &mut line, MAX_HEADERS.saturating_sub(head.len()))?;
            if n == 0 {
                return Err(HttpError::BadHeaders("flux clos avant la fin des en-têtes".into()));
            }
            let is_blank = line == b"\r\n"; // terminaison STRICTE CRLF (un LF nu n'est pas une ligne vide)
            head.extend_from_slice(&line);
            if is_blank {
                break;
            }
            if head.len() >= MAX_HEADERS {
                return Err(HttpError::BadHeaders("bloc d'en-têtes trop volumineux".into()));
            }
        }
        let (method, raw_target, headers) = parse_head(&head)?;
        // 2. cadre du corps — STRICT (anti-désync derrière un proxy non-conforme) :
        //    Transfer-Encoding non supporté (rejeté, pas ignoré) ; Content-Length unique et numérique.
        if headers.iter().any(|(k, _)| k == "transfer-encoding") {
            return Err(HttpError::BadHeaders("transfer-encoding non supporté".into()));
        }
        let content_length = parse_content_length(&headers)?;
        if content_length > MAX_BODY {
            return Err(HttpError::BodyTooLarge);
        }
        // Lecture INCRÉMENTALE : un envoi lent/court ne réserve jamais le tampon de la taille déclarée.
        let body = read_body_capped(&mut reader, content_length)?;
        Self::assemble(method, raw_target, headers, body)
    }

    /// Parse une requête **complète** déjà en mémoire (tête + corps) — pratique pour les tests.
    pub fn parse(buf: &[u8]) -> Result<HttpRequest, HttpError> {
        // Terminateur d'en-tête STRICT (RFC 7230 : CRLFCRLF ; pas de LF nu).
        let head_end = find_subsequence(buf, b"\r\n\r\n").map(|i| i + 4).ok_or_else(|| HttpError::BadHeaders("terminateur d'en-tête \\r\\n\\r\\n absent".into()))?;
        if head_end > MAX_HEADERS {
            return Err(HttpError::BadHeaders("bloc d'en-têtes trop volumineux".into()));
        }
        let (method, raw_target, headers) = parse_head(&buf[..head_end])?;
        let body = buf[head_end..].to_vec();
        if body.len() > MAX_BODY {
            return Err(HttpError::BodyTooLarge);
        }
        Self::assemble(method, raw_target, headers, body)
    }

    fn assemble(method: HttpMethod, raw_target: String, headers: Vec<(String, String)>, body: Vec<u8>) -> Result<HttpRequest, HttpError> {
        let (raw_path, raw_query) = match raw_target.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (raw_target.as_str(), None),
        };
        let path = percent_decode(raw_path).ok_or_else(|| HttpError::BadPath("séquence %XX invalide".into()))?;
        // Sûreté : un chemin ne doit pas porter d'octet de contrôle / NUL (anti-injection de log, anti-piège).
        if path.bytes().any(|b| b < 0x20 || b == 0x7f) {
            return Err(HttpError::BadPath("octet de contrôle dans le chemin".into()));
        }
        let query = match raw_query {
            Some(q) => parse_query(q),
            None => Vec::new(),
        };
        Ok(HttpRequest { method, raw_target, path, query, headers, body })
    }
}

/// Une réponse HTTP — corps binaire (HTML, JSON, ou un tar).
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub content_type: String,
    /// En-têtes additionnels (ex. `Content-Disposition` pour un téléchargement).
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl HttpResponse {
    pub fn html(status: u16, body: impl Into<Vec<u8>>) -> HttpResponse {
        HttpResponse { status, content_type: "text/html; charset=utf-8".into(), headers: Vec::new(), body: body.into() }
    }
    pub fn json(status: u16, body: impl Into<Vec<u8>>) -> HttpResponse {
        HttpResponse { status, content_type: "application/json; charset=utf-8".into(), headers: Vec::new(), body: body.into() }
    }
    /// Une réponse de téléchargement (le bundle tar).
    pub fn download(filename: &str, content_type: &str, body: Vec<u8>) -> HttpResponse {
        // Le nom de fichier est borné à un alphabet sûr par l'appelant ; on l'insère tel quel (déjà validé).
        HttpResponse {
            status: 200,
            content_type: content_type.into(),
            headers: vec![("content-disposition".into(), format!("attachment; filename=\"{filename}\""))],
            body,
        }
    }
    /// Une redirection (303 See Other — après un POST d'action, pour éviter le re-POST).
    pub fn redirect(location: &str) -> HttpResponse {
        HttpResponse { status: 303, content_type: "text/plain; charset=utf-8".into(), headers: vec![("location".into(), location.to_string())], body: Vec::new() }
    }

    /// Sérialise la réponse en octets HTTP/1.1 (connexion fermée après — pas de keep-alive).
    pub fn to_bytes(&self) -> Vec<u8> {
        let reason = reason_phrase(self.status);
        let mut out = Vec::new();
        out.extend_from_slice(format!("HTTP/1.1 {} {}\r\n", self.status, reason).as_bytes());
        out.extend_from_slice(format!("Content-Type: {}\r\n", self.content_type).as_bytes());
        out.extend_from_slice(format!("Content-Length: {}\r\n", self.body.len()).as_bytes());
        out.extend_from_slice(b"Connection: close\r\n");
        // Durcissement minimal de navigateur (la console est locale ; ces en-têtes ne coûtent rien).
        out.extend_from_slice(b"X-Content-Type-Options: nosniff\r\n");
        out.extend_from_slice(b"Referrer-Policy: no-referrer\r\n");
        for (k, v) in &self.headers {
            // Anti-injection d'en-tête : on refuse silencieusement une valeur à CR/LF (ne devrait pas arriver).
            if !v.contains('\r') && !v.contains('\n') {
                out.extend_from_slice(format!("{k}: {v}\r\n").as_bytes());
            }
        }
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(&self.body);
        out
    }
}

// ── parsing interne ──

/// Le résultat du parsing de la tête : méthode, cible brute, en-têtes (noms en minuscules).
type ParsedHead = (HttpMethod, String, Vec<(String, String)>);

fn parse_head(head: &[u8]) -> Result<ParsedHead, HttpError> {
    let text = std::str::from_utf8(head).map_err(|_| HttpError::BadHeaders("en-têtes non-UTF-8".into()))?;
    // RFC 7230 : terminaison STRICTE CRLF — un LF nu (non précédé de CR) est un vecteur de désync, rejeté.
    let segments: Vec<&str> = text.split("\r\n").collect();
    if segments.iter().any(|l| l.contains('\n')) {
        return Err(HttpError::BadHeaders("terminaison de ligne non-CRLF (LF nu)".into()));
    }
    let mut lines = segments.into_iter().filter(|l| !l.is_empty());
    let request_line = lines.next().ok_or_else(|| HttpError::BadRequestLine("vide".into()))?;
    if request_line.len() > MAX_REQUEST_LINE {
        return Err(HttpError::BadRequestLine("trop longue".into()));
    }
    let mut parts = request_line.split(' ');
    let method = HttpMethod::parse(parts.next().ok_or_else(|| HttpError::BadRequestLine("méthode absente".into()))?);
    let target = parts.next().ok_or_else(|| HttpError::BadRequestLine("cible absente".into()))?.to_string();
    // (la version HTTP, parts.next(), est ignorée — on répond toujours en 1.1, connexion fermée)
    if !target.starts_with('/') {
        return Err(HttpError::BadRequestLine("cible non absolue".into()));
    }
    let mut headers = Vec::new();
    for line in lines {
        let (k, v) = line.split_once(':').ok_or_else(|| HttpError::BadHeaders(format!("en-tête sans « : » : {line}")))?;
        headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
    }
    Ok((method, target, headers))
}

/// Décode `%XX` (et laisse les autres octets tels quels). `+` n'est PAS décodé ici (réservé au query).
/// Rend `None` sur un `%` mal formé. Le résultat doit rester de l'UTF-8 valide.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if i + 2 >= bytes.len() {
                    return None;
                }
                let hi = hex_val(bytes[i + 1])?;
                let lo = hex_val(bytes[i + 2])?;
                out.push(hi * 16 + lo);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Décode une valeur de query : `+` → espace, puis `%XX`.
fn query_decode(s: &str) -> Option<String> {
    percent_decode(&s.replace('+', " "))
}

/// Parse un corps `application/x-www-form-urlencoded` — même grammaire que la query (`+`/`%XX` décodés).
pub fn parse_urlencoded(body: &str) -> Vec<(String, String)> {
    parse_query(body)
}

fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .filter_map(|pair| {
            let (k, v) = match pair.split_once('=') {
                Some((k, v)) => (k, v),
                None => (pair, ""),
            };
            // Une paire à clé/valeur indécodable est ignorée (jamais un panic).
            Some((query_decode(k)?, query_decode(v)?))
        })
        .collect()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn find_subsequence(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

/// Cadre le `Content-Length` (anti-smuggling) : **zéro ou un** en-tête, valeur en **chiffres ASCII purs**
/// (on rejette les doublons, le `+`, les espaces internes, et tout non-numérique). Un `Content-Length`
/// ambigu est exactement la désync front-end/back-end qu'on refuse plutôt que de tolérer.
fn parse_content_length(headers: &[(String, String)]) -> Result<usize, HttpError> {
    let values: Vec<&str> = headers.iter().filter(|(k, _)| k == "content-length").map(|(_, v)| v.as_str()).collect();
    match values.as_slice() {
        [] => Ok(0),
        [v] => {
            let s = v.trim();
            if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                return Err(HttpError::BadHeaders("Content-Length non numérique".into()));
            }
            s.parse::<usize>().map_err(|_| HttpError::BadHeaders("Content-Length hors borne".into()))
        }
        _ => Err(HttpError::BadHeaders("Content-Length en double".into())),
    }
}

/// Lit exactement `len` octets de corps, **par morceaux** — jamais de pré-allocation de la taille déclarée
/// (un envoi lent/court ne réserve pas un tampon géant ; `len` est déjà borné par [`MAX_BODY`]).
fn read_body_capped<R: Read>(reader: &mut R, len: usize) -> Result<Vec<u8>, HttpError> {
    let mut body = Vec::with_capacity(len.min(64 * 1024));
    let mut buf = [0u8; 16 * 1024];
    while body.len() < len {
        let want = (len - body.len()).min(buf.len());
        match reader.read(&mut buf[..want]) {
            Ok(0) => return Err(HttpError::Io("flux clos avant la fin du corps".into())),
            Ok(n) => body.extend_from_slice(&buf[..n]),
            Err(e) => return Err(HttpError::Io(e.to_string())),
        }
    }
    Ok(body)
}

/// Lit une ligne (jusqu'à `\n` inclus) en bornant la taille — anti-DoS (une ligne d'en-tête infinie).
fn read_line_capped<R: BufRead>(reader: &mut R, out: &mut Vec<u8>, cap: usize) -> Result<usize, HttpError> {
    let mut total = 0;
    loop {
        let mut byte = [0u8; 1];
        match reader.read(&mut byte) {
            Ok(0) => return Ok(total),
            Ok(_) => {
                out.push(byte[0]);
                total += 1;
                if byte[0] == b'\n' {
                    return Ok(total);
                }
                if total > cap {
                    return Err(HttpError::BadHeaders("ligne d'en-tête trop longue".into()));
                }
            }
            Err(e) => return Err(HttpError::Io(e.to_string())),
        }
    }
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        303 => "See Other",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        _ => "Status",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_get_with_query() {
        let raw = b"GET /cases/SUP-2024?fw=gdpr&limit=10 HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer tok\r\n\r\n";
        let r = HttpRequest::parse(raw).unwrap();
        assert_eq!(r.method, HttpMethod::Get);
        assert_eq!(r.path, "/cases/SUP-2024");
        assert_eq!(r.query_get("fw"), Some("gdpr"));
        assert_eq!(r.query_get("limit"), Some("10"));
        assert_eq!(r.header("authorization"), Some("Bearer tok"));
        assert_eq!(r.header("AUTHORIZATION"), Some("Bearer tok"), "recherche insensible à la casse");
    }

    #[test]
    fn parses_a_post_with_body() {
        let raw = b"POST /cases/X/action HTTP/1.1\r\nContent-Length: 7\r\n\r\nhello!!";
        let r = HttpRequest::parse(raw).unwrap();
        assert_eq!(r.method, HttpMethod::Post);
        assert_eq!(r.body, b"hello!!");
    }

    #[test]
    fn percent_and_plus_decoding() {
        let raw = b"GET /cases/a%20b?q=x%2By+z HTTP/1.1\r\n\r\n";
        let r = HttpRequest::parse(raw).unwrap();
        assert_eq!(r.path, "/cases/a b", "%20 décodé dans le chemin");
        assert_eq!(r.query_get("q"), Some("x+y z"), "%2B → +, + → espace dans le query");
    }

    #[test]
    fn rejects_malformed_inputs_without_panic() {
        assert!(matches!(HttpRequest::parse(b"GET /a%2 HTTP/1.1\r\n\r\n"), Err(HttpError::BadPath(_))), "%XX tronqué");
        assert!(matches!(HttpRequest::parse(b"GET relative HTTP/1.1\r\n\r\n"), Err(HttpError::BadRequestLine(_))), "cible non absolue");
        assert!(matches!(HttpRequest::parse(b"no-blank-line"), Err(HttpError::BadHeaders(_))), "pas de fin de tête");
        // Un octet de contrôle décodé dans le chemin est refusé.
        assert!(matches!(HttpRequest::parse(b"GET /a%00b HTTP/1.1\r\n\r\n"), Err(HttpError::BadPath(_))), "NUL dans le chemin");
    }

    #[test]
    fn response_round_trips_and_strips_header_injection() {
        let resp = HttpResponse::html(200, "<h1>ok</h1>");
        let bytes = resp.to_bytes();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(s.contains("Content-Type: text/html"));
        assert!(s.contains("Content-Length: 11"));
        assert!(s.contains("nosniff"));
        assert!(s.ends_with("<h1>ok</h1>"));
        // Un en-tête à CR/LF injecté est éliminé (anti response-splitting).
        let mut bad = HttpResponse::html(200, "x");
        bad.headers.push(("x-evil".into(), "a\r\nSet-Cookie: pwned".into()));
        let s2 = String::from_utf8_lossy(&bad.to_bytes()).to_string();
        assert!(!s2.contains("Set-Cookie"), "valeur d'en-tête à CR/LF rejetée");
    }

    #[test]
    fn strict_framing_rejects_smuggling_primitives() {
        use std::io::Cursor;
        // Content-Length en DOUBLE → refusé (jamais « le premier gagne »).
        let dup = b"POST /x HTTP/1.1\r\nContent-Length: 3\r\nContent-Length: 7\r\n\r\nAAABBBB";
        assert!(matches!(HttpRequest::read_from(Cursor::new(&dup[..])), Err(HttpError::BadHeaders(_))), "CL en double refusé");
        // Content-Length « +7 » non canonique → refusé.
        let plus = b"POST /x HTTP/1.1\r\nContent-Length: +7\r\n\r\nAAAAAAA";
        assert!(matches!(HttpRequest::read_from(Cursor::new(&plus[..])), Err(HttpError::BadHeaders(_))), "CL non numérique refusé");
        // Transfer-Encoding non supporté → REFUSÉ (pas ignoré, ce qui serait une désync chunked).
        let te = b"POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n";
        assert!(matches!(HttpRequest::read_from(Cursor::new(&te[..])), Err(HttpError::BadHeaders(_))), "transfer-encoding refusé");
        // LF nu dans un en-tête (terminaison non-CRLF) → refusé.
        let mixed = b"GET / HTTP/1.1\r\nX: a\nY: b\r\n\r\n";
        assert!(matches!(HttpRequest::parse(mixed), Err(HttpError::BadHeaders(_))), "LF nu refusé");
        // Cas sain : une requête CRLF bien formée passe toujours.
        assert!(HttpRequest::read_from(Cursor::new(&b"POST /x HTTP/1.1\r\nContent-Length: 2\r\n\r\nhi"[..])).is_ok());
    }

    #[test]
    fn body_cap_is_enforced_before_allocation() {
        // Anti-DoS : un Content-Length déclaré au-delà de MAX_BODY est refusé par read_from AVANT toute
        // allocation (on ne réserve jamais le tampon géant). On teste le chemin streaming (read_from).
        let head = format!("POST /x HTTP/1.1\r\nContent-Length: {}\r\n\r\n", MAX_BODY + 1);
        let cursor = std::io::Cursor::new(head.into_bytes());
        assert!(matches!(HttpRequest::read_from(cursor), Err(HttpError::BodyTooLarge)), "Content-Length géant refusé sans allouer");
    }
}
