//! 运行期设置：模型接口与网络代理，落盘到数据目录的 config.json。
//!
//! 优先级：config.json > 环境变量（含 .env）> 内置默认值。

use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// 常见的本地代理端口，按「最可能是 HTTP 代理」的顺序排列。
const COMMON_PORTS: [u16; 8] = [10809, 7890, 7897, 10808, 8889, 1080, 8080, 8888];

fn env_or(name: &str, fallback: &str) -> String {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => fallback.to_string(),
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    /// auto：环境变量 → 系统代理 → 本地常见端口；manual：只用 proxy_url；off：直连。
    pub proxy_mode: String,
    pub proxy_url: String,
    pub no_proxy: String,
}

impl Default for Settings {
    fn default() -> Self {
        dotenvy::dotenv().ok();
        Self {
            api_base: env_or("LLM_API_BASE", "https://api.deepseek.com/v1"),
            api_key: std::env::var("LLM_API_KEY").unwrap_or_default(),
            model: env_or("LLM_MODEL", "deepseek-chat"),
            proxy_mode: env_or("STUDYFORGE_PROXY_MODE", "auto"),
            proxy_url: std::env::var("HTTPS_PROXY")
                .or_else(|_| std::env::var("HTTP_PROXY"))
                .unwrap_or_default(),
            no_proxy: std::env::var("NO_PROXY").unwrap_or_default(),
        }
    }
}

impl Settings {
    /// 读 config.json；文件不存在或字段缺失时回落到环境变量与默认值。
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Settings::default(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// 当前实际生效的代理地址（None 表示直连）。
    pub fn resolved_proxy(&self) -> Option<String> {
        match self.proxy_mode.as_str() {
            "off" => None,
            "manual" => {
                let url = self.proxy_url.trim();
                if url.is_empty() {
                    None
                } else {
                    Some(url.to_string())
                }
            }
            _ => {
                // 每一项都要确认本机端口真的在听：系统里常留着指向已停代理的设置，
                // 照着连只会让所有请求失败，不如直接回退直连。
                let env = env_proxy();
                if !env.is_empty() && proxy_reachable(&env) {
                    return Some(env);
                }
                if let Some(system) = windows_system_proxy() {
                    if proxy_reachable(&system) {
                        return Some(system);
                    }
                }
                scan_ports()
                    .first()
                    .map(|port| format!("http://127.0.0.1:{port}"))
            }
        }
    }

    /// 按设置构建 HTTP 客户端（自动带上代理）。
    pub fn build_http(&self, timeout_secs: u64) -> Result<reqwest::Client> {
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .user_agent("Zhizhi/0.1");
        if let Some(url) = self.resolved_proxy() {
            let mut proxy = reqwest::Proxy::all(&url)
                .with_context(|| format!("代理地址不合法：{url}"))?;
            if !self.no_proxy.trim().is_empty() {
                proxy = proxy.no_proxy(reqwest::NoProxy::from_string(&self.no_proxy));
            }
            builder = builder.proxy(proxy);
        }
        builder.build().context("构建 HTTP 客户端失败")
    }
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProxyInfo {
    /// 环境变量里的代理
    pub env_proxy: String,
    /// Windows 系统代理（设置 → 网络和 Internet → 代理）
    pub system_proxy: String,
    /// 本地正在监听的常见代理端口
    pub open_ports: Vec<u16>,
    /// 建议使用的代理地址
    pub suggested: String,
    /// 当前设置下实际会用的代理
    pub effective: String,
    pub mode: String,
}

/// 检测代理：环境变量、系统设置、本地端口都看一遍。
pub fn detect(settings: &Settings) -> ProxyInfo {
    let env = env_proxy();
    let system = windows_system_proxy().unwrap_or_default();
    let ports = scan_ports();
    let suggested = if !env.is_empty() {
        env.clone()
    } else if !system.is_empty() {
        system.clone()
    } else if let Some(port) = ports.first() {
        format!("http://127.0.0.1:{port}")
    } else {
        String::new()
    };
    ProxyInfo {
        env_proxy: env,
        system_proxy: system,
        open_ports: ports,
        suggested,
        effective: settings.resolved_proxy().unwrap_or_default(),
        mode: settings.proxy_mode.clone(),
    }
}

fn env_proxy() -> String {
    for name in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        if let Ok(value) = std::env::var(name) {
            let value = value.trim();
            if !value.is_empty() {
                return normalize_proxy(value);
            }
        }
    }
    String::new()
}

fn normalize_proxy(value: &str) -> String {
    let value = value.trim();
    if value.contains("://") {
        value.to_string()
    } else {
        format!("http://{value}")
    }
}

/// 本机代理才做可达性检查：远端代理可能挡探测，误判反而更糟。
fn proxy_reachable(url: &str) -> bool {
    let without_scheme = url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_start_matches("socks5://")
        .trim_start_matches("socks5h://");
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);
    let Some((host, port)) = authority.rsplit_once(':') else {
        return true;
    };
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return true;
    }
    match port.parse::<u16>() {
        Ok(port) => port_open(port),
        Err(_) => true,
    }
}

pub fn scan_ports() -> Vec<u16> {
    COMMON_PORTS
        .iter()
        .copied()
        .filter(|port| port_open(*port))
        .collect()
}

fn port_open(port: u16) -> bool {
    let address = match format!("127.0.0.1:{port}").to_socket_addrs() {
        Ok(mut iter) => match iter.next() {
            Some(addr) => addr,
            None => return false,
        },
        Err(_) => return false,
    };
    TcpStream::connect_timeout(&address, Duration::from_millis(160)).is_ok()
}

#[cfg(target_os = "windows")]
pub fn windows_system_proxy() -> Option<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
        .ok()?;
    let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
    if enabled == 0 {
        return None;
    }
    let server: String = key.get_value("ProxyServer").ok()?;
    let server = server.trim();
    if server.is_empty() {
        return None;
    }
    // 可能是 "host:port"，也可能是 "http=host:port;https=host:port"
    let picked = if server.contains('=') {
        server
            .split(';')
            .filter_map(|part| {
                let part = part.trim();
                part.strip_prefix("https=").or_else(|| part.strip_prefix("http="))
            })
            .next()
            .unwrap_or(server)
            .to_string()
    } else {
        server.to_string()
    };
    Some(normalize_proxy(&picked))
}

#[cfg(not(target_os = "windows"))]
pub fn windows_system_proxy() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_bare_host_port() {
        assert_eq!(normalize_proxy("127.0.0.1:10809"), "http://127.0.0.1:10809");
        assert_eq!(normalize_proxy("http://a:1"), "http://a:1");
    }

    #[test]
    fn manual_mode_overrides_detection() {
        let settings = Settings {
            proxy_mode: "manual".into(),
            proxy_url: "http://127.0.0.1:9999".into(),
            ..Default::default()
        };
        assert_eq!(settings.resolved_proxy().as_deref(), Some("http://127.0.0.1:9999"));

        let off = Settings { proxy_mode: "off".into(), ..Default::default() };
        assert!(off.resolved_proxy().is_none());
    }

    #[test]
    fn dead_local_proxy_is_skipped() {
        assert!(!proxy_reachable("http://127.0.0.1:59999"), "没在听的本地端口不该被采用");
        assert!(proxy_reachable("http://203.0.113.9:8080"), "远端代理不做探测");
        assert!(proxy_reachable("http://127.0.0.1"), "没写端口交给 reqwest");
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let parsed: Settings = serde_json::from_str("{\"apiKey\":\"k\"}").unwrap();
        assert_eq!(parsed.api_key, "k");
        assert_eq!(parsed.model, Settings::default().model);
    }
}