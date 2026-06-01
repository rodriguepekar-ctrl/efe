//! `DecodeMode` and `DecodedOutput` ↔ proto conversion.

use std::num::NonZeroU32;

use crate::proto::v1 as proto;
use efe_core::{DecodeMode, DecodedOutput, EfeError, Result};

const DEFAULT_MAX_TOKENS: u32 = 256;

/// Convert a proto `DecodeMode` into domain.
///
/// If `None`, defaults to [`DecodeMode::default`] (text, 256 tokens).
pub fn mode_from_proto(p: Option<proto::DecodeMode>) -> Result<DecodeMode> {
    let Some(m) = p else {
        return Ok(DecodeMode::default());
    };
    match m.mode {
        Some(proto::decode_mode::Mode::Text(t)) => {
            let tokens = if t.max_tokens == 0 {
                DEFAULT_MAX_TOKENS
            } else {
                t.max_tokens
            };
            let nz = NonZeroU32::new(tokens)
                .ok_or_else(|| EfeError::InvalidConfig("max_tokens must be > 0".into()))?;
            Ok(DecodeMode::Text { max_tokens: nz })
        }
        Some(proto::decode_mode::Mode::PassThrough(_)) => Ok(DecodeMode::PassThrough),
        None => Ok(DecodeMode::default()),
    }
}

/// Convert a domain `DecodedOutput` into proto.
#[must_use]
pub fn output_to_proto(o: DecodedOutput) -> proto::DecodedOutput {
    match o {
        DecodedOutput::Text(s) => proto::DecodedOutput {
            payload: Some(proto::decoded_output::Payload::Text(s)),
        },
        DecodedOutput::Raw(v) => proto::DecodedOutput {
            payload: Some(proto::decoded_output::Payload::Raw(proto::RawCentroid {
                values: v,
            })),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_defaults_to_text() {
        let m = mode_from_proto(None).unwrap();
        assert!(matches!(m, DecodeMode::Text { .. }));
    }

    #[test]
    fn explicit_text_respected() {
        let p = proto::DecodeMode {
            mode: Some(proto::decode_mode::Mode::Text(proto::TextMode {
                max_tokens: 512,
            })),
        };
        let m = mode_from_proto(Some(p)).unwrap();
        match m {
            DecodeMode::Text { max_tokens } => assert_eq!(max_tokens.get(), 512),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn zero_tokens_falls_back_to_default() {
        let p = proto::DecodeMode {
            mode: Some(proto::decode_mode::Mode::Text(proto::TextMode {
                max_tokens: 0,
            })),
        };
        let m = mode_from_proto(Some(p)).unwrap();
        match m {
            DecodeMode::Text { max_tokens } => assert_eq!(max_tokens.get(), DEFAULT_MAX_TOKENS),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn pass_through_maps() {
        let p = proto::DecodeMode {
            mode: Some(proto::decode_mode::Mode::PassThrough(
                proto::PassThroughMode {},
            )),
        };
        let m = mode_from_proto(Some(p)).unwrap();
        assert!(matches!(m, DecodeMode::PassThrough));
    }

    #[test]
    fn output_text_maps() {
        let p = output_to_proto(DecodedOutput::Text("hello".into()));
        match p.payload {
            Some(proto::decoded_output::Payload::Text(s)) => assert_eq!(s, "hello"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn output_raw_maps() {
        let p = output_to_proto(DecodedOutput::Raw(vec![1.0, 2.0]));
        match p.payload {
            Some(proto::decoded_output::Payload::Raw(r)) => {
                assert_eq!(r.values, vec![1.0, 2.0]);
            }
            _ => panic!("expected raw"),
        }
    }
}
