// SPDX-License-Identifier: UNLICENSED
//! Tests edge cases parsing SSE pour les 2 providers.

#![cfg(any(feature = "anthropic", feature = "gemini"))]

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
