// Gather configured secrets once, then redact their occurrences in every exported log.
#[derive(Default)]
pub struct SecretRedactor {
    secrets: Vec<String>,
}

impl SecretRedactor {
    pub fn add_config(&mut self, text: &str) -> Result<(), String> {
        let text = text.trim_start_matches('\u{feff}');
        let value = serde_json::from_str::<serde_json::Value>(text).ok().or_else(|| {
            toml::from_str::<toml::Value>(text).ok().and_then(|value| serde_json::to_value(value).ok())
        }).ok_or_else(|| "Невозможно безопасно прочитать настройки для экспорта".to_string())?;
        self.collect(&value, false);
        self.secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        self.secrets.dedup();
        Ok(())
    }

    fn collect(&mut self, value: &serde_json::Value, sensitive: bool) {
        match value {
            serde_json::Value::Object(values) => for (key, value) in values {
                let key = key.to_ascii_lowercase();
                let secret = sensitive || matches!(key.as_str(), "keys" | "key" | "token" | "access_token" | "secret" | "password") || key.starts_with("api_key");
                self.collect(value, secret);
            },
            serde_json::Value::Array(values) => for value in values { self.collect(value, sensitive); },
            serde_json::Value::String(secret) if sensitive && !secret.is_empty() => {
                self.secrets.push(secret.clone());
                if let Ok(quoted) = serde_json::to_string(secret) {
                    self.secrets.push(quoted[1..quoted.len() - 1].to_string());
                }
            }
            _ => {}
        }
    }

    pub fn redact(&self, text: &str) -> String {
        self.secrets.iter().fold(text.to_string(), |out, secret| out.replace(secret, "[скрыто]"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_multiline_short_and_legacy_keys_are_redacted_across_logs() {
        let mut redactor = SecretRedactor::default();
        redactor.add_config("[[llm.providers]]\nkeys = [\n 'arbitrary-token',\n 'tiny'\n]\n").unwrap();
        redactor.add_config(r#"{"api_keys":{"picovoice":"legacy-secret"},"voice":"jarvis"}"#).unwrap();
        let output = redactor.redact("arbitrary-token tiny legacy-secret jarvis");
        assert_eq!(output, "[скрыто] [скрыто] [скрыто] jarvis");
        assert!(redactor.add_config("keys = ['broken-secret'").is_err());
    }
}

#[cfg(test)]
mod agent_tests {
    #[test]
    fn gateway_tokens_are_redacted_in_every_exported_file() {
        let mut redactor=super::SecretRedactor::default();
        redactor.add_config("[agent.openclaw]\napi_key='arbitrary-gateway-credential'\n").unwrap();
        assert_eq!(redactor.redact("failure arbitrary-gateway-credential in gateway"),"failure [скрыто] in gateway");
    }
}
