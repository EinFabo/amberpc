//! Servers that speak the OpenAI audio API. Known services have a fixed address and a list
//! of models; an own server (Speaches, LocalAI, whisper.cpp, ...) gets free fields.

use reqwest::Url;

pub struct SpeechModel {
    pub id: &'static str,
    pub label: &'static str,
    pub note: &'static str,
}

pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    /// Empty for an own server.
    pub base_url: &'static str,
    pub models: &'static [SpeechModel],
    /// Where to get a key.
    pub key_url: &'static str,
    pub needs_key: bool,
}

impl Provider {
    pub fn default_model(&self) -> &'static str {
        self.models.first().map(|m| m.id).unwrap_or("whisper-1")
    }
}

// Prices from the providers' public pages, October 2026.
pub const GROQ: Provider = Provider {
    id: "groq",
    name: "Groq",
    base_url: "https://api.groq.com/openai/v1",
    models: &[
        SpeechModel { id: "whisper-large-v3-turbo", label: "Whisper large-v3-turbo", note: "fast · $0.04 per hour" },
        SpeechModel { id: "whisper-large-v3", label: "Whisper large-v3", note: "a little more accurate · $0.11 per hour" },
    ],
    key_url: "https://console.groq.com/keys",
    needs_key: true,
};

pub const OPENAI: Provider = Provider {
    id: "openai",
    name: "OpenAI",
    base_url: "https://api.openai.com/v1",
    models: &[
        SpeechModel { id: "gpt-4o-mini-transcribe", label: "GPT-4o mini Transcribe", note: "$0.003 per minute" },
        SpeechModel { id: "gpt-4o-transcribe", label: "GPT-4o Transcribe", note: "most accurate · $0.006 per minute" },
        SpeechModel { id: "whisper-1", label: "Whisper", note: "$0.006 per minute" },
    ],
    key_url: "https://platform.openai.com/api-keys",
    needs_key: true,
};

pub const CUSTOM: Provider = Provider {
    id: "custom",
    name: "Own server",
    base_url: "",
    models: &[],
    key_url: "",
    needs_key: false,
};

pub const ALL: [&Provider; 3] = [&GROQ, &OPENAI, &CUSTOM];

pub fn by_id(id: &str) -> Option<&'static Provider> {
    ALL.iter().copied().find(|p| p.id == id)
}

/// The address requests go to: fixed for known services, the user's for an own server.
pub fn effective_base_url(provider: &Provider, custom: &str) -> String {
    if provider.base_url.is_empty() {
        custom.trim().trim_end_matches('/').to_string()
    } else {
        provider.base_url.to_string()
    }
}

/// https everywhere; plain http only to machines in the user's own network, where a self
/// hosted server usually has no certificate.
pub fn check_base_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw.trim()).map_err(|_| "Not a valid address (e.g. https://example.com/v1).".to_string())?;
    match url.scheme() {
        "https" => Ok(url),
        "http" if is_private_host(url.host_str()) => Ok(url),
        "http" => Err("http:// only works for servers in your own network - use https://.".into()),
        _ => Err("The address has to start with https://.".into()),
    }
}

/// Loopback, private ranges, CGNAT (Tailscale), link-local, single-label and .local names.
pub fn is_private_host(host: Option<&str>) -> bool {
    let Some(h) = host.map(|h| h.trim_matches(|c| c == '[' || c == ']').to_lowercase()) else {
        return false;
    };
    if h.is_empty() {
        return false;
    }
    if let Ok(ip) = h.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v4) => {
                let [a, b, ..] = v4.octets();
                v4.is_loopback() || v4.is_private() || v4.is_link_local() || (a == 100 && (64..=127).contains(&b))
            }
            std::net::IpAddr::V6(v6) => {
                if let Some(v4) = v6.to_ipv4_mapped() {
                    return is_private_host(Some(&v4.to_string()));
                }
                let first = v6.segments()[0];
                v6.is_loopback() || (0xfc00..=0xfdff).contains(&first) || (0xfe80..=0xfebf).contains(&first)
            }
        };
    }
    !h.contains('.') || [".local", ".lan", ".home.arpa", ".internal", ".ts.net"].iter().any(|s| h.ends_with(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_consistent() {
        for p in ALL {
            assert_eq!(by_id(p.id).map(|q| q.id), Some(p.id));
            if !p.base_url.is_empty() {
                assert!(check_base_url(p.base_url).is_ok(), "{}", p.id);
                assert!(!p.models.is_empty());
            }
        }
        assert_eq!(GROQ.default_model(), "whisper-large-v3-turbo");
        assert_eq!(CUSTOM.default_model(), "whisper-1");
    }

    #[test]
    fn effective_url() {
        assert_eq!(effective_base_url(&GROQ, "https://evil.example"), GROQ.base_url);
        assert_eq!(effective_base_url(&CUSTOM, " http://nas:8000/v1/ "), "http://nas:8000/v1");
    }

    #[test]
    fn http_only_at_home() {
        for ok in ["http://127.0.0.1:8000/v1", "http://192.168.1.20/v1", "http://nas:8000", "http://box.local/v1",
                   "http://100.100.1.2/v1", "http://[::1]:9000", "http://[fd12::1]/v1", "https://api.example.com/v1"] {
            assert!(check_base_url(ok).is_ok(), "{ok}");
        }
        for bad in ["http://api.example.com/v1", "http://8.8.8.8", "http://[2001:db8::1]/", "ftp://nas", "nonsense",
                    "http://[::ffff:8.8.8.8]/"] {
            assert!(check_base_url(bad).is_err(), "{bad}");
        }
    }
}
