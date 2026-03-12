//! Secret decode/edit helpers for the decoded workbench view.

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_yaml::{Mapping, Value};

const HEX_PREVIEW_BYTES: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedSecretValue {
    Text {
        original: String,
        current: String,
    },
    Binary {
        raw_base64: String,
        byte_len: usize,
        preview: String,
    },
    InvalidBase64 {
        raw_base64: String,
        error: String,
        replacement: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedSecretEntry {
    pub key: String,
    pub value: DecodedSecretValue,
}

impl DecodedSecretEntry {
    pub fn is_editable(&self) -> bool {
        !matches!(self.value, DecodedSecretValue::Binary { .. })
    }

    pub fn is_dirty(&self) -> bool {
        match &self.value {
            DecodedSecretValue::Text { original, current } => original != current,
            DecodedSecretValue::Binary { .. } => false,
            DecodedSecretValue::InvalidBase64 { replacement, .. } => replacement.is_some(),
        }
    }

    pub fn editable_text(&self) -> Option<&str> {
        match &self.value {
            DecodedSecretValue::Text { current, .. } => Some(current),
            DecodedSecretValue::Binary { .. } => None,
            DecodedSecretValue::InvalidBase64 { replacement, .. } => {
                replacement.as_deref().or(Some(""))
            }
        }
    }

    pub fn commit_edit(&mut self, edited: String) -> bool {
        match &mut self.value {
            DecodedSecretValue::Text { current, .. } => {
                if *current == edited {
                    false
                } else {
                    *current = edited;
                    true
                }
            }
            DecodedSecretValue::Binary { .. } => false,
            DecodedSecretValue::InvalidBase64 { replacement, .. } => {
                if replacement.as_deref() == Some(edited.as_str()) {
                    false
                } else {
                    *replacement = Some(edited);
                    true
                }
            }
        }
    }

    pub fn raw_value_for_apply(&self) -> String {
        match &self.value {
            DecodedSecretValue::Text { current, .. } => BASE64.encode(current.as_bytes()),
            DecodedSecretValue::Binary { raw_base64, .. } => raw_base64.clone(),
            DecodedSecretValue::InvalidBase64 {
                raw_base64,
                replacement,
                ..
            } => replacement
                .as_ref()
                .map(|value| BASE64.encode(value.as_bytes()))
                .unwrap_or_else(|| raw_base64.clone()),
        }
    }
}

pub fn decode_secret_yaml(yaml: &str) -> Result<Vec<DecodedSecretEntry>> {
    let parsed: Value = serde_yaml::from_str(yaml).context("invalid Secret YAML")?;
    let mapping = parsed
        .as_mapping()
        .ok_or_else(|| anyhow!("Secret YAML root must be a mapping"))?;
    let Some(data) = mapping.get(Value::String("data".to_string())) else {
        return Ok(Vec::new());
    };
    let data_mapping = data
        .as_mapping()
        .ok_or_else(|| anyhow!("Secret data must be a mapping"))?;

    data_mapping
        .iter()
        .map(|(key, value)| {
            let key = key
                .as_str()
                .ok_or_else(|| anyhow!("Secret data keys must be strings"))?
                .to_string();
            let raw_base64 = value
                .as_str()
                .ok_or_else(|| anyhow!("Secret data field '{key}' must be a base64 string"))?
                .to_string();

            let decoded = match BASE64.decode(raw_base64.as_bytes()) {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(text) => DecodedSecretValue::Text {
                        original: text.clone(),
                        current: text,
                    },
                    Err(err) => {
                        let bytes = err.into_bytes();
                        DecodedSecretValue::Binary {
                            raw_base64,
                            byte_len: bytes.len(),
                            preview: hex_preview(&bytes),
                        }
                    }
                },
                Err(err) => DecodedSecretValue::InvalidBase64 {
                    raw_base64,
                    error: err.to_string(),
                    replacement: None,
                },
            };

            Ok(DecodedSecretEntry {
                key,
                value: decoded,
            })
        })
        .collect()
}

pub fn encode_secret_yaml(yaml: &str, entries: &[DecodedSecretEntry]) -> Result<String> {
    let mut parsed: Value = serde_yaml::from_str(yaml).context("invalid Secret YAML")?;
    let root = parsed
        .as_mapping_mut()
        .ok_or_else(|| anyhow!("Secret YAML root must be a mapping"))?;

    let data_key = Value::String("data".to_string());
    let mut data = Mapping::new();
    for entry in entries {
        data.insert(
            Value::String(entry.key.clone()),
            Value::String(entry.raw_value_for_apply()),
        );
    }
    root.insert(data_key, Value::Mapping(data));

    serde_yaml::to_string(&parsed).context("failed to serialize Secret YAML")
}

fn hex_preview(bytes: &[u8]) -> String {
    let preview_len = bytes.len().min(HEX_PREVIEW_BYTES);
    let mut rendered = bytes[..preview_len]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    if bytes.len() > preview_len {
        rendered.push_str(" …");
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_secret_yaml_reads_text_entries() {
        let entries =
            decode_secret_yaml("apiVersion: v1\nkind: Secret\ndata:\n  token: aGVsbG8=\n")
                .expect("decoded entries");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "token");
        assert_eq!(entries[0].editable_text(), Some("hello"));
    }

    #[test]
    fn decode_secret_yaml_marks_binary_values() {
        let entries = decode_secret_yaml("apiVersion: v1\nkind: Secret\ndata:\n  blob: //79AA==\n")
            .expect("decoded entries");

        assert!(matches!(
            entries[0].value,
            DecodedSecretValue::Binary { .. }
        ));
    }

    #[test]
    fn decode_secret_yaml_marks_invalid_base64_values() {
        let entries =
            decode_secret_yaml("apiVersion: v1\nkind: Secret\ndata:\n  broken: not-base64%%\n")
                .expect("decoded entries");

        assert!(matches!(
            entries[0].value,
            DecodedSecretValue::InvalidBase64 { .. }
        ));
    }

    #[test]
    fn encode_secret_yaml_round_trips_edited_text() {
        let yaml = "apiVersion: v1\nkind: Secret\ndata:\n  token: aGVsbG8=\n";
        let mut entries = decode_secret_yaml(yaml).expect("decoded entries");
        assert!(entries[0].commit_edit("updated".to_string()));

        let encoded = encode_secret_yaml(yaml, &entries).expect("encoded yaml");
        assert!(encoded.contains("dXBkYXRlZA=="));
    }

    #[test]
    fn invalid_base64_entry_can_be_repaired() {
        let yaml = "apiVersion: v1\nkind: Secret\ndata:\n  broken: not-base64%%\n";
        let mut entries = decode_secret_yaml(yaml).expect("decoded entries");
        assert!(entries[0].commit_edit("fixed".to_string()));

        let encoded = encode_secret_yaml(yaml, &entries).expect("encoded yaml");
        assert!(encoded.contains("Zml4ZWQ="));
    }
}
