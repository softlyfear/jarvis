// Optional agent settings are independent of the HTTP feature and the GUI.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    #[default]
    Direct,
    Openclaw,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentConfig {
    pub backend: BackendKind,
    pub fallback_backend: String,
    pub mcp_enabled: bool,
    pub openclaw: OpenClawConfig,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            backend: BackendKind::Direct,
            fallback_backend: "direct".into(),
            mcp_enabled: false,
            openclaw: OpenClawConfig::default(),
        }
    }
}

#[derive(Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct OpenClawConfig {
    pub base_url: String,
    pub api_key: String,
    pub agent: String,
    // Empty uses the agent's configured model. Never a hardcoded provider catalogue.
    pub model: String,
    pub vision_model: String,
    pub connect_timeout_secs: u64,
    #[serde(alias = "timeout_secs")]
    pub request_timeout_secs: u64,
    pub task_timeout_secs: u64,
    pub max_tool_rounds: usize,
    pub streaming: bool,
}

impl std::fmt::Debug for OpenClawConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenClawConfig")
            .field("agent", &self.agent)
            .field("api_key", &"[скрыто]")
            .finish_non_exhaustive()
    }
}

impl Default for OpenClawConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:18789".into(),
            api_key: String::new(),
            agent: "jarvis".into(),
            model: String::new(),
            vision_model: String::new(),
            connect_timeout_secs: 3,
            request_timeout_secs: 60,
            task_timeout_secs: 300,
            max_tool_rounds: 32,
            streaming: false,
        }
    }
}

impl AgentConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !["direct", "none"].contains(&self.fallback_backend.as_str()) {
            return Err("Резервный режим: direct или none".into());
        }
        let c = &self.openclaw;
        // Tokens in URLs leak through HTTP diagnostics; plaintext is restricted to loopback.
        let (scheme, rest) = c
            .base_url
            .split_once("://")
            .ok_or("Адрес OpenClaw должен начинаться с http:// или https://")?;
        let authority = rest.split('/').next().unwrap_or("");
        let host = authority.split(':').next().unwrap_or("");
        if authority.is_empty()
            || rest.contains(['@', '?', '#'])
            || rest.chars().any(char::is_whitespace)
            || !["http", "https"].contains(&scheme)
            || (scheme == "http" && !["127.0.0.1", "localhost", "["].contains(&host))
            || (scheme == "http" && host == "[" && !authority.starts_with("[::1]"))
        {
            return Err(
                "OpenClaw: локальный HTTP или защищённый HTTPS, без ключей в адресе".into(),
            );
        }
        if c.agent.is_empty()
            || c.agent.len() > 100
            || !c
                .agent
                .chars()
                .all(|x| x.is_ascii_alphanumeric() || matches!(x, '_' | '-'))
        {
            return Err("Имя агента: латинские буквы, цифры, _ и -".into());
        }
        if c.api_key.contains(['\r', '\n'])
            || c.model.contains(['\r', '\n'])
            || c.vision_model.contains(['\r', '\n'])
        {
            return Err("Параметры OpenClaw не должны содержать переносы строк".into());
        }
        if !(1..=30).contains(&c.connect_timeout_secs)
            || !(3..=600).contains(&c.request_timeout_secs)
            || !(3..=1800).contains(&c.task_timeout_secs)
            || !(1..=128).contains(&c.max_tool_rounds)
        {
            return Err("OpenClaw: недопустимый таймаут или предел действий".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_configs_remain_direct_and_openclaw_round_trips() {
        assert_eq!(
            crate::assistant_config::parse("[llm]\nenabled=true")
                .unwrap()
                .agent
                .backend,
            BackendKind::Direct
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("assistant.toml");
        let mut settings = crate::assistant_config::read_editable_from(&path).unwrap();
        settings.agent.backend = BackendKind::Openclaw;
        settings.agent.openclaw.api_key = "random-gateway-token".into();
        settings.agent.openclaw.model = "provider/model".into();
        settings.agent.openclaw.streaming = true;
        crate::assistant_config::write_editable_to(&path, &settings).unwrap();
        assert_eq!(
            crate::assistant_config::read_editable_from(&path)
                .unwrap()
                .agent,
            settings.agent
        );
    }
    #[test]
    fn credentials_in_urls_plaintext_remote_hosts_and_invalid_limits_are_rejected() {
        for url in [
            "http://evil.example",
            "http://127.0.0.1.evil",
            "https://token@host/",
            "https://host/?token=secret",
            "file:///tmp/api",
        ] {
            let mut c = AgentConfig::default();
            c.openclaw.base_url = url.into();
            assert!(c.validate().is_err(), "{}", url);
        }
        let mut c = AgentConfig::default();
        c.openclaw.max_tool_rounds = 0;
        assert!(c.validate().is_err());
        c.openclaw.max_tool_rounds = 32;
        c.openclaw.request_timeout_secs = u64::MAX;
        assert!(c.validate().is_err());
        let parsed = crate::assistant_config::parse("[agent.openclaw]\ntimeout_secs=75").unwrap();
        assert_eq!(parsed.agent.openclaw.request_timeout_secs, 75);
    }
}
