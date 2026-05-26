// SPDX-License-Identifier: UNLICENSED
//! Tests edge cases parsing SSE pour les 3 providers (Anthropic, Gemini, OpenAI).

#![cfg(any(feature = "anthropic", feature = "gemini", feature = "openai"))]

use bytes::Bytes;
use futures::stream;

fn str_to_byte_stream(
    s: &'static str,
) -> impl futures::Stream<Item = Result<Bytes, reqwest::Error>> + Unpin {
    // 1 chunk = 1 octet pour stress test
    let chunks: Vec<_> = s
        .as_bytes()
        .chunks(1)
        .map(|c| Ok::<_, reqwest::Error>(Bytes::copy_from_slice(c)))
        .collect();
    Box::pin(stream::iter(chunks))
}

fn str_to_byte_stream_big(
    s: &'static str,
) -> impl futures::Stream<Item = Result<Bytes, reqwest::Error>> + Unpin {
    // 1 chunk = 1KB pour cas réaliste
    let chunks: Vec<_> = s
        .as_bytes()
        .chunks(1024)
        .map(|c| Ok::<_, reqwest::Error>(Bytes::copy_from_slice(c)))
        .collect();
    Box::pin(stream::iter(chunks))
}

#[cfg(feature = "anthropic")]
mod anthropic {
    use super::*;
    use efe_coordinator::llm::providers::streaming::parse_anthropic_sse;

    #[tokio::test]
    async fn handles_single_byte_chunks() {
        // Stress test : chaque octet arrive séparément.
        let sse = "event: content_block_delta\ndata: {\"delta\":{\"text\":\"OK\"}}\n\nevent: message_stop\ndata: {}\n\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_byte_stream(sse);
        let res = parse_anthropic_sse(stream, tx).await.unwrap();
        let mut got = vec![];
        while let Some(c) = rx.recv().await {
            got.push(c.token);
        }
        assert_eq!(got, vec!["OK"]);
        assert_eq!(res.text, "OK");
    }

    #[tokio::test]
    async fn handles_malformed_json_data_skipped() {
        // 1ère data malformée → skip ; 2ème data OK → token reçu.
        let sse = "event: content_block_delta\ndata: {not valid json}\n\nevent: content_block_delta\ndata: {\"delta\":{\"text\":\"valid\"}}\n\nevent: message_stop\ndata: {}\n\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_byte_stream_big(sse);
        let res = parse_anthropic_sse(stream, tx).await.unwrap();
        let mut got = vec![];
        while let Some(c) = rx.recv().await {
            got.push(c.token);
        }
        assert_eq!(got, vec!["valid"]);
        assert_eq!(res.text, "valid");
    }

    #[tokio::test]
    async fn handles_empty_stream() {
        let sse = "";
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_byte_stream_big(sse);
        let res = parse_anthropic_sse(stream, tx).await.unwrap();
        assert!(rx.recv().await.is_none());
        assert_eq!(res.text, "");
        assert_eq!(res.output_tokens, 0);
    }

    #[tokio::test]
    async fn handles_unknown_event_types_ignored() {
        let sse = "event: ping\ndata: {}\n\nevent: content_block_delta\ndata: {\"delta\":{\"text\":\"X\"}}\n\nevent: unknown_event\ndata: {}\n\nevent: message_stop\ndata: {}\n\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_byte_stream_big(sse);
        let _ = parse_anthropic_sse(stream, tx).await.unwrap();
        let mut got = vec![];
        while let Some(c) = rx.recv().await {
            got.push(c.token);
        }
        assert_eq!(got, vec!["X"]);
    }

    #[tokio::test]
    async fn handles_usage_in_message_delta() {
        let sse = "event: content_block_delta\ndata: {\"delta\":{\"text\":\"a\"}}\n\nevent: content_block_delta\ndata: {\"delta\":{\"text\":\"b\"}}\n\nevent: message_delta\ndata: {\"delta\":{},\"usage\":{\"output_tokens\":42}}\n\nevent: message_stop\ndata: {}\n\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_byte_stream_big(sse);
        let res = parse_anthropic_sse(stream, tx).await.unwrap();
        // Drain
        while rx.recv().await.is_some() {}
        // output_tokens du message_delta (42) doit dominer le count token.
        assert_eq!(res.output_tokens, 42);
    }
}

#[cfg(feature = "gemini")]
mod gemini {
    use super::*;
    use efe_coordinator::llm::providers::streaming::parse_gemini_sse;

    #[tokio::test]
    async fn handles_single_byte_chunks() {
        let sse = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"OK\"}]}}]}\n\ndata: {\"candidates\":[{\"finishReason\":\"STOP\"}]}\n\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_byte_stream(sse);
        let _ = parse_gemini_sse(stream, tx).await.unwrap();
        let mut got = vec![];
        while let Some(c) = rx.recv().await {
            got.push(c.token);
        }
        assert_eq!(got, vec!["OK"]);
    }

    #[tokio::test]
    async fn handles_malformed_json_data_skipped() {
        let sse = "data: {garbage}\n\ndata: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"clean\"}]}}]}\n\ndata: {\"candidates\":[{\"finishReason\":\"STOP\"}]}\n\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_byte_stream_big(sse);
        let _ = parse_gemini_sse(stream, tx).await.unwrap();
        let mut got = vec![];
        while let Some(c) = rx.recv().await {
            got.push(c.token);
        }
        assert_eq!(got, vec!["clean"]);
    }

    #[tokio::test]
    async fn handles_empty_stream() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_byte_stream_big("");
        let res = parse_gemini_sse(stream, tx).await.unwrap();
        assert!(rx.recv().await.is_none());
        assert_eq!(res.text, "");
    }

    #[tokio::test]
    async fn captures_usage_metadata_in_final_chunk() {
        let sse = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hi\"}]}}]}\n\ndata: {\"candidates\":[{\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":3,\"candidatesTokenCount\":1}}\n\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_byte_stream_big(sse);
        let res = parse_gemini_sse(stream, tx).await.unwrap();
        while rx.recv().await.is_some() {}
        assert_eq!(res.input_tokens, 3);
        assert!(res.output_tokens >= 1);
    }
}

#[cfg(feature = "openai")]
mod openai {
    use super::*;
    use efe_coordinator::llm::providers::streaming::parse_openai_sse;

    #[tokio::test]
    async fn handles_single_byte_chunks() {
        // Stress test : 1 octet par chunk, séparateur \n\n.
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"OK\"}}]}\n\ndata: [DONE]\n\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_byte_stream(sse);
        let res = parse_openai_sse(stream, tx).await.unwrap();
        let mut got = vec![];
        while let Some(c) = rx.recv().await {
            got.push(c.token);
        }
        assert_eq!(got, vec!["OK"]);
        assert_eq!(res.text, "OK");
    }

    #[tokio::test]
    async fn handles_done_sentinel_terminates_stream() {
        // Pas de finish_reason explicite, mais [DONE] arrête le parsing.
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"hello \"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"world\"}}]}\n\ndata: [DONE]\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"NEVER\"}}]}\n\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_byte_stream_big(sse);
        let res = parse_openai_sse(stream, tx).await.unwrap();
        let mut got = vec![];
        while let Some(c) = rx.recv().await {
            got.push(c.token);
        }
        assert_eq!(got, vec!["hello ", "world"]);
        assert!(!res.text.contains("NEVER"));
        assert_eq!(res.text, "hello world");
    }

    #[tokio::test]
    async fn handles_malformed_json_data_skipped() {
        let sse = "data: {garbage}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"clean\"}}]}\n\ndata: [DONE]\n\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_byte_stream_big(sse);
        let res = parse_openai_sse(stream, tx).await.unwrap();
        let mut got = vec![];
        while let Some(c) = rx.recv().await {
            got.push(c.token);
        }
        assert_eq!(got, vec!["clean"]);
        assert_eq!(res.text, "clean");
    }

    #[tokio::test]
    async fn handles_empty_stream() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_byte_stream_big("");
        let res = parse_openai_sse(stream, tx).await.unwrap();
        assert!(rx.recv().await.is_none());
        assert_eq!(res.text, "");
        assert_eq!(res.output_tokens, 0);
    }

    #[tokio::test]
    async fn captures_usage_from_final_chunk() {
        // OpenAI envoie `usage` dans le dernier chunk avant [DONE] si
        // stream_options.include_usage=true.
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"b\"}}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2,\"total_tokens\":7}}\n\ndata: [DONE]\n\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_byte_stream_big(sse);
        let res = parse_openai_sse(stream, tx).await.unwrap();
        while rx.recv().await.is_some() {}
        assert_eq!(res.text, "ab");
        assert_eq!(res.input_tokens, 5);
        assert_eq!(res.output_tokens, 2);
    }

    #[tokio::test]
    async fn handles_crlf_separators() {
        // HTTP utilise typiquement \r\n\r\n comme séparateur SSE.
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"crlf\"}}]}\r\n\r\ndata: [DONE]\r\n\r\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_byte_stream_big(sse);
        let res = parse_openai_sse(stream, tx).await.unwrap();
        let mut got = vec![];
        while let Some(c) = rx.recv().await {
            got.push(c.token);
        }
        assert_eq!(got, vec!["crlf"]);
        assert_eq!(res.text, "crlf");
    }

    #[tokio::test]
    async fn handles_finish_reason_followed_by_done() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_byte_stream_big(sse);
        let res = parse_openai_sse(stream, tx).await.unwrap();
        while rx.recv().await.is_some() {}
        assert_eq!(res.text, "x");
    }

    #[tokio::test]
    async fn handles_role_only_first_chunk() {
        // 1er chunk : delta.role="assistant" (pas de content) → ignoré silencieusement.
        let sse = "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"go\"}}]}\n\ndata: [DONE]\n\n";
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = str_to_byte_stream_big(sse);
        let res = parse_openai_sse(stream, tx).await.unwrap();
        let mut got = vec![];
        while let Some(c) = rx.recv().await {
            got.push(c.token);
        }
        assert_eq!(got, vec!["go"]);
        assert_eq!(res.text, "go");
    }
}
