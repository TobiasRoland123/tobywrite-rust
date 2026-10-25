use std::fmt;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum Encoding {
    #[default]
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

impl fmt::Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8 BOM",
            Self::Utf16Le => "UTF-16 LE",
            Self::Utf16Be => "UTF-16 BE",
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
        }
    }
}

impl fmt::Display for LineEnding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Lf => "LF",
            Self::CrLf => "CRLF",
            Self::Cr => "CR",
        })
    }
}

/// Text is normalized to LF in memory. The original BOM and newline style
/// are restored when saving.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TextFile {
    pub text: String,
    pub encoding: Encoding,
    pub line_ending: LineEnding,
}

impl TextFile {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        // UTF-32 LE shares its first two bytes with UTF-16 LE.
        if bytes.starts_with(&[0xff, 0xfe, 0, 0]) || bytes.starts_with(&[0, 0, 0xfe, 0xff]) {
            bail!("UTF-32 files are not supported. Open UTF-8 or UTF-16 text instead.");
        }
        let (encoding, decoded) = if let Some(body) = bytes.strip_prefix(&[0xff, 0xfe]) {
            (Encoding::Utf16Le, decode_utf16(body, true)?)
        } else if let Some(body) = bytes.strip_prefix(&[0xfe, 0xff]) {
            (Encoding::Utf16Be, decode_utf16(body, false)?)
        } else if let Some(body) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
            (
                Encoding::Utf8Bom,
                std::str::from_utf8(body)
                    .context("Invalid UTF-8 text")?
                    .to_owned(),
            )
        } else {
            (
                Encoding::Utf8,
                std::str::from_utf8(bytes)
                    .context("This file is not supported UTF-8 or BOM-marked UTF-16 text")?
                    .to_owned(),
            )
        };
        if decoded
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\u{c}'))
        {
            bail!("This file contains binary control characters and cannot be opened as text.");
        }
        let line_ending = if decoded.contains("\r\n") {
            LineEnding::CrLf
        } else if decoded.contains('\r') {
            LineEnding::Cr
        } else {
            LineEnding::Lf
        };
        Ok(Self {
            text: decoded.replace("\r\n", "\n").replace('\r', "\n"),
            encoding,
            line_ending,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        // Clipboard providers can supply CRLF even though GTK edits LF text.
        // Normalize first so preserving CRLF never produces a CRCRLF sequence.
        let normalized = self.text.replace("\r\n", "\n").replace('\r', "\n");
        let output = normalized.replace('\n', self.line_ending.as_str());
        match self.encoding {
            Encoding::Utf8 => output.into_bytes(),
            Encoding::Utf8Bom => {
                let mut bytes = vec![0xef, 0xbb, 0xbf];
                bytes.extend_from_slice(output.as_bytes());
                bytes
            }
            Encoding::Utf16Le | Encoding::Utf16Be => {
                let little = self.encoding == Encoding::Utf16Le;
                let mut bytes = if little {
                    vec![0xff, 0xfe]
                } else {
                    vec![0xfe, 0xff]
                };
                for unit in output.encode_utf16() {
                    bytes.extend_from_slice(&if little {
                        unit.to_le_bytes()
                    } else {
                        unit.to_be_bytes()
                    });
                }
                bytes
            }
        }
    }
}

fn decode_utf16(bytes: &[u8], little: bool) -> Result<String> {
    if !bytes.len().is_multiple_of(2) {
        bail!("Invalid UTF-16 text: incomplete code unit");
    }
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            if little {
                u16::from_le_bytes(*pair)
            } else {
                u16::from_be_bytes(*pair)
            }
        })
        .collect();
    String::from_utf16(&units).context("Invalid UTF-16 text: unpaired surrogate")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_and_boms_round_trip_in_every_encoding_and_newline_style() {
        for encoding in [
            Encoding::Utf8,
            Encoding::Utf8Bom,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            for line_ending in [LineEnding::Lf, LineEnding::CrLf, LineEnding::Cr] {
                let original = TextFile {
                    text: "# København\nこんにちは 👩🏽‍💻\n\n".into(),
                    encoding,
                    line_ending,
                };
                assert_eq!(TextFile::decode(&original.encode()).unwrap(), original);
            }
        }
        assert_eq!(TextFile::decode(&[]).unwrap(), TextFile::default());
    }

    #[test]
    fn edits_retain_line_endings() {
        let mut file = TextFile::decode(b"one\r\ntwo\r\n").unwrap();
        assert_eq!(file.text, "one\ntwo\n");
        file.text.push_str("three\n");
        assert_eq!(file.encode(), b"one\r\ntwo\r\nthree\r\n");
        let mixed = TextFile::decode(b"one\rtwo\r\nthree\n").unwrap();
        assert_eq!(mixed.line_ending, LineEnding::CrLf);
        assert_eq!(mixed.text, "one\ntwo\nthree\n");
    }

    #[test]
    fn clipboard_newlines_do_not_double_carriage_returns_on_save() {
        let file = TextFile {
            text: "pasted\r\nWindows\rand Unix\n".into(),
            encoding: Encoding::Utf8,
            line_ending: LineEnding::CrLf,
        };
        assert_eq!(file.encode(), b"pasted\r\nWindows\r\nand Unix\r\n");
    }

    #[test]
    fn invalid_encodings_and_binary_are_rejected_without_loss() {
        for bytes in [
            &[0xc3, 0x28][..],
            &[b'a', 0, b'b'],
            &[0x7f],
            &[0x1b, b'['],
            &[0xff, 0xfe, 0, 0, 0x41, 0, 0, 0],
            &[0, 0, 0xfe, 0xff],
            &[0xff, 0xfe, 0x41],
            &[0xff, 0xfe, 0, 0xd8],
            &[0xfe, 0xff, 0xdc, 0],
            &[0xff, 0xfe, 0, 0xdc],
        ] {
            assert!(TextFile::decode(bytes).is_err(), "accepted {bytes:?}");
        }
    }

    #[test]
    fn bom_only_documents_keep_their_encoding() {
        for bytes in [&[0xef, 0xbb, 0xbf][..], &[0xff, 0xfe], &[0xfe, 0xff]] {
            assert_eq!(TextFile::decode(bytes).unwrap().encode(), bytes);
        }
    }
}
