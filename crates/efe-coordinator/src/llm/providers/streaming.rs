// SPDX-License-Identifier: UNLICENSED
//! Parseurs SSE pour les 3 protocoles distincts (ADR-P3R1-005).
//!
//! - Anthropic : `event: <type>\ndata: <json>\n\n`
//! - Gemini    : `data: <json>\n\n` (pas de `event:` prefix)
//! - OpenAI    : `data: <json>\n\n` ... `data: [DONE]\n\n` (terminator)

#[cfg(any(feature = "anthropic", feature = "gemini", feature = "openai"))]
use chrono::Utc;
#[cfg(any(feature = "anthropic", feature = "gemini", feature = "openai"))]
use tokio::sync::mpsc;

#[cfg(any(feature = "anthropic", feature = "gemini", feature = "openai"))]
use crate::error::CoordResult;
#[cfg(any(feature = "anthropic", feature = "gemini", feature = "openai"))]
use crate::llm::TokenChunk;

/// Résultat d'un parse SSE complet.
#[cfg(any(feature = "anthropic", feature = "gemini", feature = "openai"))]
#[derive(Debug, Clone, Default)]
pub struct SseStreamResult {
    /// Texte concaténé de tous les chunks.
    pub text: String,
    /// Total tokens output (depuis le message_delta Anthropic ou usageMetadata Gemini).
    pub output_tokens: u32,
    /// Tokens input (si le provider l'a inclus dans le stream).
    pub input_tokens: u32,
}

/// Parse un stream Anthropic SSE (format `event: <type>\ndata: <json>\n\n`).
///
/// Émet chaque token sur `chunk_tx` au fur et à mesure.
#[cfg(feature = "anthropic")]
pub async fn parse_anthropic_sse(
    mut stream: impl futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin,
    chunk_tx: mpsc::Sender<TokenChunk>,
) -> CoordResult<SseStreamResult> {
    use futures::StreamExt;

    let mut buffer = String::new();
    let mut current_event: Option<String>;
    let mut current_data = String::new();
    // Silence unused_assignments avant le 1er reset in-loop.
    let _ = &current_data;
    let mut text = String::new();
    let mut token_index = 0_u64;
    let mut output_tokens = 0_u32;
    let mut input_tokens = 0_u32;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            crate::error::CoordError::LlmProvider {
                provider: "claude".into(),
                reason: format!("transport during stream: {e}"),
            }
        })?;
        let s = std::str::from_utf8(&chunk).unwrap_or("");
        buffer.push_str(s);

        // Consume tant qu'il y a des messages complets (sep `\n\n` ou `\r\n\r\n`).
        while let Some((idx, sep_len)) = find_sse_message_end(&buffer) {
            let raw_msg = buffer[..idx].to_string();
            buffer.drain(..idx + sep_len);
            current_event = None;
            current_data.clear();
            for line in raw_msg.lines() {
                if let Some(e) = line.strip_prefix("event: ") {
                    current_event = Some(e.to_string());
                } else if let Some(d) = line.strip_prefix("data: ") {
                    current_data.push_str(d);
                }
            }
            if current_data.is_empty() {
                continue;
            }
            let payload: serde_json::Value =
                match serde_json::from_str(&current_data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
            match current_event.as_deref() {
                Some("content_block_delta") => {
                    if let Some(t) = payload["delta"]["text"].as_str() {
                        let _ = chunk_tx
                            .send(TokenChunk {
                                token: t.to_string(),
                                index: token_index,
                                emitted_at: Utc::now(),
                            })
                            .await;
                        text.push_str(t);
                        token_index += 1;
                    }
                }
                Some("message_start") => {
                    if let Some(n) = payload["message"]["usage"]["input_tokens"].as_u64() {
                        input_tokens = n as u32;
                    }
                }
                Some("message_delta") => {
                    if let Some(n) = payload["usage"]["output_tokens"].as_u64() {
                        output_tokens = n as u32;
                    }
                }
                Some("message_stop") => break,
                _ => {} // ignored : content_block_start, content_block_stop, ping
            }
        }
    }

    Ok(SseStreamResult {
        text,
        output_tokens: output_tokens.max(token_index as u32),
        input_tokens,
    })
}

/// Trouve la fin du prochain message SSE.  Tolère `\r\n\r\n` (style HTTP)
/// ET `\n\n` (style Unix).  Retourne `(start_offset, separator_len)`.
#[cfg(any(feature = "anthropic", feature = "gemini", feature = "openai"))]
fn find_sse_message_end(buf: &str) -> Option<(usize, usize)> {
    // `\r\n\r\n` d'abord (plus spécifique).
    if let Some(i) = buf.find("\r\n\r\n") {
        // S'il y a aussi `\n\n` AVANT le `\r\n\r\n`, prendre celui-là.
        if let Some(j) = buf.find("\n\n") {
            if j < i {
                return Some((j, 2));
            }
        }
        return Some((i, 4));
    }
    buf.find("\n\n").map(|i| (i, 2))
}

/// Parse un stream Gemini SSE (format `data: <json>` séparé par `\n\n` ou `\r\n\r\n`).
///
/// Le parser :
/// 1. Accepte `data: <json>` (avec espace après `:`).
/// 2. Accepte `data:<json>` (sans espace, défensif).
/// 3. Si le buffer démarre par `[` ou `{` (JSON array streamé sans alt=sse),
///    fallback : parse comme JSON top-level et émet les éléments.
#[cfg(feature = "gemini")]
pub async fn parse_gemini_sse(
    mut stream: impl futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin,
    chunk_tx: mpsc::Sender<TokenChunk>,
) -> CoordResult<SseStreamResult> {
    use futures::StreamExt;

    let mut buffer = String::new();
    let mut text = String::new();
    let mut token_index = 0_u64;
    let mut output_tokens = 0_u32;
    let mut input_tokens = 0_u32;
    let mut finished = false;

    let process_payload =
        |payload: serde_json::Value,
         text: &mut String,
         token_index: &mut u64,
         input_tokens: &mut u32,
         output_tokens: &mut u32,
         finished: &mut bool,
         chunk_tx: &mpsc::Sender<TokenChunk>| {
            if let Some(parts) = payload["candidates"][0]["content"]["parts"].as_array() {
                for part in parts {
                    if let Some(t) = part["text"].as_str() {
                        if !t.is_empty() {
                            let _ = chunk_tx.try_send(TokenChunk {
                                token: t.to_string(),
                                index: *token_index,
                                emitted_at: Utc::now(),
                            });
                            text.push_str(t);
                            *token_index += 1;
                        }
                    }
                }
            }
            if let Some(n) = payload["usageMetadata"]["promptTokenCount"].as_u64() {
                *input_tokens = n as u32;
            }
            if let Some(n) = payload["usageMetadata"]["candidatesTokenCount"].as_u64() {
                *output_tokens = n as u32;
            }
            if payload["candidates"][0]["finishReason"].as_str().is_some() {
                *finished = true;
            }
        };

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            crate::error::CoordError::LlmProvider {
                provider: "gemini".into(),
                reason: format!("transport during stream: {e}"),
            }
        })?;
        let s = std::str::from_utf8(&chunk).unwrap_or("");
        buffer.push_str(s);

        // Format 1 : SSE `data: <json>` séparé par `\n\n` ou `\r\n\r\n`.
        while let Some((idx, sep_len)) = find_sse_message_end(&buffer) {
            let raw_msg = buffer[..idx].to_string();
            buffer.drain(..idx + sep_len);
            for line in raw_msg.lines() {
                let data = line
                    .strip_prefix("data: ")
                    .or_else(|| line.strip_prefix("data:"))
                    .map(str::trim_start);
                let data = match data {
                    Some(d) => d,
                    None => continue,
                };
                let payload: serde_json::Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                process_payload(
                    payload,
                    &mut text,
                    &mut token_index,
                    &mut input_tokens,
                    &mut output_tokens,
                    &mut finished,
                    &chunk_tx,
                );
            }
            if finished {
                break;
            }
        }
        if finished {
            break;
        }
    }

    // Format 2 (fallback) : si le buffer entier ressemble à un JSON array ou
    // un object — cas où Gemini renvoie `[{...},{...}]` sans alt=sse pris en
    // compte.  On parse au mieux.
    if token_index == 0 && !buffer.trim().is_empty() {
        let trimmed = buffer.trim();
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            // Array streamé : itère sur les éléments.
            if let Some(arr) = val.as_array() {
                for item in arr {
                    process_payload(
                        item.clone(),
                        &mut text,
                        &mut token_index,
                        &mut input_tokens,
                        &mut output_tokens,
                        &mut finished,
                        &chunk_tx,
                    );
                }
            } else if val.is_object() {
                process_payload(
                    val,
                    &mut text,
                    &mut token_index,
                    &mut input_tokens,
                    &mut output_tokens,
                    &mut finished,
                    &chunk_tx,
                );
            }
        }
    }

    Ok(SseStreamResult {
        text,
        output_tokens: output_tokens.max(token_index as u32),
        input_tokens,
    })
}

/// Parse un stream OpenAI SSE.
///
/// Format :
/// ```text
/// data: {"id":"...","choices":[{"delta":{"role":"assistant"}}]}\n\n
/// data: {"id":"...","choices":[{"delta":{"content":"Hello"}}]}\n\n
/// data: {"id":"...","choices":[{"delta":{"content":" world"}}]}\n\n
/// data: {"choices":[{"delta":{},"finish_reason":"stop"}]}\n\n
/// data: [DONE]\n\n
/// ```
///
/// - Pas de `event:` prefix (différent d'Anthropic).
/// - Le stream termine sur `data: [DONE]` (sentinel literal).
/// - Le `usage` peut apparaître dans le DERNIER chunk avant `[DONE]` si
///   `stream_options.include_usage=true` est défini dans la requête.
#[cfg(feature = "openai")]
pub async fn parse_openai_sse(
    mut stream: impl futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin,
    chunk_tx: mpsc::Sender<TokenChunk>,
) -> CoordResult<SseStreamResult> {
    use futures::StreamExt;

    let mut buffer = String::new();
    let mut text = String::new();
    let mut token_index = 0_u64;
    let mut output_tokens = 0_u32;
    let mut input_tokens = 0_u32;
    let mut finished = false;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| crate::error::CoordError::LlmProvider {
            provider: "gpt".into(),
            reason: format!("transport during stream: {e}"),
        })?;
        let s = std::str::from_utf8(&chunk).unwrap_or("");
        buffer.push_str(s);

        while let Some((idx, sep_len)) = find_sse_message_end(&buffer) {
            let raw_msg = buffer[..idx].to_string();
            buffer.drain(..idx + sep_len);

            for line in raw_msg.lines() {
                // OpenAI : `data: ` (avec espace) typiquement, mais on est
                // défensif : `data:` sans espace aussi.
                let data = line
                    .strip_prefix("data: ")
                    .or_else(|| line.strip_prefix("data:"))
                    .map(str::trim_start);
                let data = match data {
                    Some(d) => d,
                    None => continue,
                };
                // Sentinel terminal — fin du stream propre.
                if data == "[DONE]" {
                    finished = true;
                    break;
                }
                let payload: serde_json::Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                // `choices[0].delta.content` est le texte du chunk.
                if let Some(t) = payload["choices"][0]["delta"]["content"].as_str() {
                    if !t.is_empty() {
                        let _ = chunk_tx
                            .send(TokenChunk {
                                token: t.to_string(),
                                index: token_index,
                                emitted_at: Utc::now(),
                            })
                            .await;
                        text.push_str(t);
                        token_index += 1;
                    }
                }
                // `usage` apparaît typiquement dans le DERNIER chunk si
                // include_usage activé dans stream_options.
                if let Some(n) = payload["usage"]["prompt_tokens"].as_u64() {
                    input_tokens = n as u32;
                }
                if let Some(n) = payload["usage"]["completion_tokens"].as_u64() {
                    output_tokens = n as u32;
                }
                // finish_reason présent → stream achevé sémantiquement.
                if payload["choices"][0]["finish_reason"].as_str().is_some() {
                    // On continue jusqu'à `[DONE]` pour récupérer le usage final.
                }
            }
            if finished {
                break;
            }
        }
        if finished {
            break;
        }
    }

    Ok(SseStreamResult {
        text,
        output_tokens: output_tokens.max(token_index as u32),
        input_tokens,
    })
}

#[cfg(test)]
mod tests {
    #[cfg(any(feature = "anthropic", feature = "openai"))]
    use super::*;

    /// Helper : convertit un &str SSE en stream de Bytes pour les tests.
    #[cfg(any(feature = "anthropic", feature = "gemini", feature = "openai"))]
    fn str_to_stream(
        s: &'static str,
    ) -> impl futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin {
        // On chunk en blocs de 16 bytes pour simuler des arrivées partielles.
        let chunks: Vec<_> = s
            .as_bytes()
            .chunks(16)
            .map(|c| Ok::<_, reqwest::Error>(bytes::Bytes::copy_from_slice(c)))
            .collect();
        Box::pin(futures::stream::iter(chunks))
    }

    #[cfg(feature = "anthropic")]
    #[tokio::test]
    async fn anthropic_sse_emits_token_deltas_in_order() {
        let sse = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12}}}\n\nevent: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Paris\"}}\n\nevent: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" is\"}}\n\nevent: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" the capital.\"}}\n\nevent: message_delta\ndata: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let stream = str_to_stream(sse);
        let res = parse_anthropic_sse(stream, tx).await.unwrap();
        let mut tokens = vec![];
        while let Some(c) = rx.recv().await {
            tokens.push(c.token);
        }
        assert_eq!(tokens, vec!["Paris", " is", " the capital."]);
        assert_eq!(res.text, "Paris is the capital.");
        assert_eq!(res.input_tokens, 12);
        assert_eq!(res.output_tokens, 5);
    }

    #[cfg(feature = "anthropic")]
    #[tokio::test]
    async fn anthropic_sse_partial_chunks_recovered() {
        // Même SSE qu'au-dessus mais arrivant en chunks de 16 octets.
        let sse = "event: content_block_delta\ndata: {\"delta\":{\"text\":\"X\"}}\n\nevent: content_block_delta\ndata: {\"delta\":{\"text\":\"Y\"}}\n\nevent: message_stop\ndata: {}\n\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_stream(sse);
        let _res = parse_anthropic_sse(stream, tx).await.unwrap();
        let mut tokens = vec![];
        while let Some(c) = rx.recv().await {
            tokens.push(c.token);
        }
        assert_eq!(tokens, vec!["X", "Y"]);
    }

    #[cfg(feature = "openai")]
    #[tokio::test]
    async fn openai_sse_emits_content_deltas() {
        let sse = "data: {\"id\":\"chatcmpl-A\",\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\ndata: {\"id\":\"chatcmpl-A\",\"choices\":[{\"delta\":{\"content\":\"Paris\"}}]}\n\ndata: {\"id\":\"chatcmpl-A\",\"choices\":[{\"delta\":{\"content\":\" is\"}}]}\n\ndata: {\"id\":\"chatcmpl-A\",\"choices\":[{\"delta\":{\"content\":\" the capital.\"}}]}\n\ndata: {\"id\":\"chatcmpl-A\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":5}}\n\ndata: [DONE]\n\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let stream = str_to_stream(sse);
        let res = parse_openai_sse(stream, tx).await.unwrap();
        let mut tokens = vec![];
        while let Some(c) = rx.recv().await {
            tokens.push(c.token);
        }
        assert_eq!(tokens, vec!["Paris", " is", " the capital."]);
        assert_eq!(res.text, "Paris is the capital.");
        assert_eq!(res.input_tokens, 12);
        assert_eq!(res.output_tokens, 5);
    }

    #[cfg(feature = "openai")]
    #[tokio::test]
    async fn openai_sse_done_sentinel_terminates() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"A\"}}]}\n\ndata: [DONE]\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"IGNORED_AFTER_DONE\"}}]}\n\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_stream(sse);
        let res = parse_openai_sse(stream, tx).await.unwrap();
        let mut tokens = vec![];
        while let Some(c) = rx.recv().await {
            tokens.push(c.token);
        }
        assert_eq!(tokens, vec!["A"]);
        assert_eq!(res.text, "A");
    }

    #[cfg(feature = "openai")]
    #[tokio::test]
    async fn openai_sse_partial_chunks() {
        // Stream chunké à 16 octets — vérifie le buffer reassembly.
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"X\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"Y\"}}]}\n\ndata: [DONE]\n\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_stream(sse);
        let _ = parse_openai_sse(stream, tx).await.unwrap();
        let mut tokens = vec![];
        while let Some(c) = rx.recv().await {
            tokens.push(c.token);
        }
        assert_eq!(tokens, vec!["X", "Y"]);
    }

    #[cfg(feature = "gemini")]
    #[tokio::test]
    async fn gemini_sse_emits_parts_text() {
        let sse = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}]}\n\ndata: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\" world\"}]}}]}\n\ndata: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\".\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":3}}\n\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_stream(sse);
        let res = parse_gemini_sse(stream, tx).await.unwrap();
        let mut tokens = vec![];
        while let Some(c) = rx.recv().await {
            tokens.push(c.token);
        }
        assert_eq!(tokens, vec!["Hello", " world", "."]);
        assert_eq!(res.text, "Hello world.");
        assert_eq!(res.input_tokens, 10);
        assert_eq!(res.output_tokens, 3);
    }
}
