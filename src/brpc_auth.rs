//! Bitcoin RPC credential auto-detection module.
//!
//! This module provides automatic detection of bitcoind RPC credentials
//! through multiple methods in priority order:
//! 1. Explicit smaug_brpc_user + smaug_brpc_pass
//! 2. Explicit smaug_brpc_cookie_dir
//! 3. listconfigs RPC for bitcoin-rpc* options
//! 4. Auto-detect cookie at standard paths
//! 5. Parse ~/.bitcoin/bitcoin.conf
//! 6. Graceful startup with warning (returns None)

use bitcoincore_rpc::Auth;
use home::home_dir;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

/// Configuration for connecting to bitcoind RPC.
#[derive(Debug, Clone)]
pub struct BrpcConfig {
    pub host: String,
    pub port: u16,
    pub auth: Auth,
}

/// Result of credential detection - either configured or unconfigured with a message.
#[derive(Debug)]
pub enum DetectionResult {
    /// Successfully detected credentials.
    Configured(BrpcConfig),
    /// No credentials found - plugin should start in unconfigured mode.
    Unconfigured(String),
}

/// Detects bitcoind RPC configuration using multiple fallback methods.
///
/// # Priority Order
/// 1. Explicit `smaug_brpc_user` + `smaug_brpc_pass` options
/// 2. Explicit `smaug_brpc_cookie_dir` option
/// 3. `listconfigs` RPC for `bitcoin-rpc*` options from CLN
/// 4. Auto-detect cookie at standard paths based on network
/// 5. Parse `~/.bitcoin/bitcoin.conf` for rpcuser/rpcpassword
/// 6. Return Unconfigured with helpful message
pub async fn detect_brpc_config(
    brpc_host: &str,
    brpc_port_opt: Option<i64>,
    brpc_user_opt: Option<String>,
    brpc_pass_opt: Option<String>,
    brpc_cookie_dir_opt: Option<String>,
    network: &str,
    rpc_file: &Path,
) -> Result<DetectionResult, anyhow::Error> {
    // Priority 1: Explicit smaug_brpc_user + smaug_brpc_pass
    if let Some(user) = brpc_user_opt {
        if let Some(pass) = brpc_pass_opt.clone() {
            let port = resolve_port(brpc_port_opt, network);
            log::debug!("Using explicit smaug_brpc_user/pass credentials");
            return Ok(DetectionResult::Configured(BrpcConfig {
                host: brpc_host.to_string(),
                port,
                auth: Auth::UserPass(user, pass),
            }));
        } else {
            return Err(anyhow::anyhow!(
                "specified `smaug_brpc_user` but did not specify `smaug_brpc_pass`"
            ));
        }
    }

    // Priority 2: Explicit smaug_brpc_cookie_dir
    if let Some(cookie_dir) = brpc_cookie_dir_opt {
        let cookie_path = PathBuf::from(&cookie_dir).join(".cookie");
        if cookie_path.exists() {
            let port = resolve_port(brpc_port_opt, network);
            log::debug!(
                "Using explicit cookie file from smaug_brpc_cookie_dir: {}",
                cookie_path.display()
            );
            return Ok(DetectionResult::Configured(BrpcConfig {
                host: brpc_host.to_string(),
                port,
                auth: Auth::CookieFile(cookie_path),
            }));
        } else {
            return Err(anyhow::anyhow!(
                "Nonexistent cookie file specified in smaug_brpc_cookie_dir: {}",
                cookie_path.display()
            ));
        }
    }

    // Priority 3: listconfigs RPC for bitcoin-rpc* options
    if let Some(config) = try_listconfigs(brpc_host, brpc_port_opt, network, rpc_file).await? {
        log::debug!("Using credentials from CLN listconfigs (bitcoin-rpc* options)");
        return Ok(DetectionResult::Configured(config));
    }

    // Priority 4: Auto-detect cookie at standard paths
    if let Some(config) = try_standard_cookie_path(brpc_host, brpc_port_opt, network)? {
        log::debug!("Using cookie file at standard path");
        return Ok(DetectionResult::Configured(config));
    }

    // Priority 5: Parse ~/.bitcoin/bitcoin.conf
    if let Some(config) = try_bitcoin_conf(brpc_host, brpc_port_opt, network)? {
        log::debug!("Using credentials from ~/.bitcoin/bitcoin.conf");
        return Ok(DetectionResult::Configured(config));
    }

    // Priority 6: Graceful startup with warning
    let help_message = format!(
        "No bitcoind RPC credentials found. Smaug will start but cannot function until configured.\n\
        \n\
        To configure bitcoind access, use one of these methods:\n\
        \n\
        1. Set explicit credentials in CLN config:\n\
           smaug_brpc_user=<rpcuser>\n\
           smaug_brpc_pass=<rpcpassword>\n\
           smaug_brpc_port=<port>  # optional, defaults based on network\n\
        \n\
        2. Point to cookie file directory:\n\
           smaug_brpc_cookie_dir=/path/to/bitcoin/datadir\n\
        \n\
        3. Ensure CLN has bitcoin-rpcuser/bitcoin-rpcpassword set\n\
        \n\
        4. Use standard cookie file location (~/.bitcoin/[network]/.cookie)\n\
        \n\
        5. Add rpcuser/rpcpassword to ~/.bitcoin/bitcoin.conf"
    );

    log::warn!("{}", help_message);
    Ok(DetectionResult::Unconfigured(help_message))
}

/// Resolves the RPC port based on explicit option or network defaults.
fn resolve_port(port_opt: Option<i64>, network: &str) -> u16 {
    match port_opt {
        Some(p) => p as u16,
        None => match network {
            "regtest" => 18443,
            "signet" | "mutinynet" => 38332,
            "testnet" => 18332,
            _ => 8332, // mainnet (bitcoin)
        },
    }
}

/// Tries to get credentials from CLN's listconfigs RPC (bitcoin-rpc* options).
///
/// Uses raw JSON-RPC because the bitcoin-rpc* options are dynamically registered
/// by the bcli plugin and not included in the typed ListconfigsConfigs struct.
async fn try_listconfigs(
    brpc_host: &str,
    brpc_port_opt: Option<i64>,
    network: &str,
    rpc_file: &Path,
) -> Result<Option<BrpcConfig>, anyhow::Error> {
    // Connect to the CLN Unix socket
    let mut stream = match UnixStream::connect(rpc_file).await {
        Ok(s) => s,
        Err(e) => {
            log::debug!("Could not connect to CLN RPC socket for listconfigs: {}", e);
            return Ok(None);
        }
    };

    // Send a raw JSON-RPC listconfigs request
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "listconfigs",
        "params": {}
    });
    let request_str = request.to_string();

    if let Err(e) = stream.write_all(request_str.as_bytes()).await {
        log::debug!("Failed to send listconfigs request: {}", e);
        return Ok(None);
    }

    // Read the response
    let mut response_buf = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                response_buf.extend_from_slice(&buf[..n]);
                // Check if we have a complete JSON response
                if let Ok(v) = serde_json::from_slice::<Value>(&response_buf) {
                    // Parse the response
                    return parse_listconfigs_response(&v, brpc_host, brpc_port_opt, network);
                }
            }
            Err(e) => {
                log::debug!("Failed to read listconfigs response: {}", e);
                return Ok(None);
            }
        }
    }

    // Try to parse whatever we got
    match serde_json::from_slice::<Value>(&response_buf) {
        Ok(v) => parse_listconfigs_response(&v, brpc_host, brpc_port_opt, network),
        Err(e) => {
            log::debug!("Failed to parse listconfigs response: {}", e);
            Ok(None)
        }
    }
}

/// Parses the listconfigs JSON response to extract bitcoin-rpc* options.
fn parse_listconfigs_response(
    response: &Value,
    brpc_host: &str,
    brpc_port_opt: Option<i64>,
    network: &str,
) -> Result<Option<BrpcConfig>, anyhow::Error> {
    let configs = match response.get("result").and_then(|r| r.get("configs")) {
        Some(c) => c,
        None => {
            log::debug!("listconfigs response missing 'result.configs'");
            return Ok(None);
        }
    };

    // Extract bitcoin-rpc* values from the JSON configs
    let rpc_user = configs
        .get("bitcoin-rpcuser")
        .and_then(|v| v.get("value_str"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());

    let rpc_password = configs
        .get("bitcoin-rpcpassword")
        .and_then(|v| v.get("value_str"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());

    let rpc_connect = configs
        .get("bitcoin-rpcconnect")
        .and_then(|v| v.get("value_str"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());

    let rpc_port = configs
        .get("bitcoin-rpcport")
        .and_then(|v| v.get("value_int"))
        .and_then(|i| i.as_i64());

    // If we have user and password, use them
    if let (Some(user), Some(pass)) = (rpc_user, rpc_password) {
        let host = rpc_connect.unwrap_or_else(|| brpc_host.to_string());
        let port = rpc_port
            .or(brpc_port_opt)
            .map(|p| p as u16)
            .unwrap_or_else(|| resolve_port(None, network));
        log::debug!(
            "Found bitcoin-rpcuser/password in listconfigs, host={}, port={}",
            host,
            port
        );
        return Ok(Some(BrpcConfig {
            host,
            port,
            auth: Auth::UserPass(user, pass),
        }));
    }

    log::debug!("No bitcoin-rpcuser/password found in listconfigs");
    Ok(None)
}

/// Tries to find cookie file at standard Bitcoin Core paths.
///
/// Network to path mapping:
/// - bitcoin (mainnet): ~/.bitcoin/.cookie
/// - testnet: ~/.bitcoin/testnet3/.cookie
/// - regtest: ~/.bitcoin/regtest/.cookie
/// - signet: ~/.bitcoin/signet/.cookie
fn try_standard_cookie_path(
    brpc_host: &str,
    brpc_port_opt: Option<i64>,
    network: &str,
) -> Result<Option<BrpcConfig>, anyhow::Error> {
    let home = match home_dir() {
        Some(h) => h,
        None => {
            log::debug!("Cannot determine home directory for cookie auto-detection");
            return Ok(None);
        }
    };

    let bitcoin_dir = home.join(".bitcoin");
    let cookie_path = match network {
        "bitcoin" => bitcoin_dir.join(".cookie"),
        "testnet" => bitcoin_dir.join("testnet3").join(".cookie"),
        "regtest" => bitcoin_dir.join("regtest").join(".cookie"),
        "signet" => bitcoin_dir.join("signet").join(".cookie"),
        "mutinynet" => bitcoin_dir.join("signet").join(".cookie"), // mutinynet uses signet
        _ => {
            log::debug!("Unknown network '{}' for cookie auto-detection", network);
            return Ok(None);
        }
    };

    if cookie_path.exists() {
        let port = resolve_port(brpc_port_opt, network);
        log::debug!("Found cookie file at: {}", cookie_path.display());
        return Ok(Some(BrpcConfig {
            host: brpc_host.to_string(),
            port,
            auth: Auth::CookieFile(cookie_path),
        }));
    }

    log::debug!(
        "Cookie file not found at standard path: {}",
        cookie_path.display()
    );
    Ok(None)
}

/// Tries to parse ~/.bitcoin/bitcoin.conf for rpcuser/rpcpassword.
///
/// Handles network-specific sections: [main], [test], [regtest], [signet]
fn try_bitcoin_conf(
    brpc_host: &str,
    brpc_port_opt: Option<i64>,
    network: &str,
) -> Result<Option<BrpcConfig>, anyhow::Error> {
    let home = match home_dir() {
        Some(h) => h,
        None => {
            log::debug!("Cannot determine home directory for bitcoin.conf parsing");
            return Ok(None);
        }
    };

    let conf_path = home.join(".bitcoin").join("bitcoin.conf");
    if !conf_path.exists() {
        log::debug!("bitcoin.conf not found at: {}", conf_path.display());
        return Ok(None);
    }

    let content = match fs::read_to_string(&conf_path) {
        Ok(c) => c,
        Err(e) => {
            log::debug!("Could not read bitcoin.conf: {}", e);
            return Ok(None);
        }
    };

    let parsed = parse_bitcoin_conf(&content, network);

    if let (Some(user), Some(pass)) = (parsed.get("rpcuser"), parsed.get("rpcpassword")) {
        let host = parsed
            .get("rpcconnect")
            .cloned()
            .unwrap_or_else(|| brpc_host.to_string());
        let port = parsed
            .get("rpcport")
            .and_then(|p| p.parse::<u16>().ok())
            .or_else(|| brpc_port_opt.map(|p| p as u16))
            .unwrap_or_else(|| resolve_port(None, network));

        return Ok(Some(BrpcConfig {
            host,
            port,
            auth: Auth::UserPass(user.clone(), pass.clone()),
        }));
    }

    log::debug!("No rpcuser/rpcpassword found in bitcoin.conf");
    Ok(None)
}

/// Parses bitcoin.conf content, handling network-specific sections.
///
/// Section names: [main] (mainnet), [test] (testnet), [regtest], [signet]
fn parse_bitcoin_conf(content: &str, network: &str) -> HashMap<String, String> {
    let mut global_values: HashMap<String, String> = HashMap::new();
    let mut section_values: HashMap<String, String> = HashMap::new();
    let mut current_section: Option<String> = None;

    // Map CLN network names to bitcoin.conf section names
    let target_section = match network {
        "bitcoin" => "main",
        "testnet" => "test",
        "regtest" => "regtest",
        "signet" | "mutinynet" => "signet",
        _ => "main",
    };

    for line in content.lines() {
        let line = line.trim();

        // Skip comments and empty lines
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Check for section header
        if line.starts_with('[') && line.ends_with(']') {
            current_section = Some(line[1..line.len() - 1].to_string());
            continue;
        }

        // Parse key=value
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().to_string();
            let value = value.trim().to_string();

            match &current_section {
                None => {
                    // Global section - applies to all networks
                    global_values.insert(key, value);
                }
                Some(section) if section == target_section => {
                    // Target network section - overrides global
                    section_values.insert(key, value);
                }
                _ => {
                    // Other network section - ignore
                }
            }
        }
    }

    // Merge: section values override global values
    for (key, value) in section_values {
        global_values.insert(key, value);
    }

    global_values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_port_explicit() {
        assert_eq!(resolve_port(Some(12345), "bitcoin"), 12345);
        assert_eq!(resolve_port(Some(12345), "regtest"), 12345);
    }

    #[test]
    fn test_resolve_port_defaults() {
        assert_eq!(resolve_port(None, "bitcoin"), 8332);
        assert_eq!(resolve_port(None, "testnet"), 18332);
        assert_eq!(resolve_port(None, "regtest"), 18443);
        assert_eq!(resolve_port(None, "signet"), 38332);
        assert_eq!(resolve_port(None, "mutinynet"), 38332);
    }

    #[test]
    fn test_parse_bitcoin_conf_global_only() {
        let content = r#"
rpcuser=alice
rpcpassword=secret123
rpcport=8332
"#;
        let parsed = parse_bitcoin_conf(content, "bitcoin");
        assert_eq!(parsed.get("rpcuser"), Some(&"alice".to_string()));
        assert_eq!(parsed.get("rpcpassword"), Some(&"secret123".to_string()));
        assert_eq!(parsed.get("rpcport"), Some(&"8332".to_string()));
    }

    #[test]
    fn test_parse_bitcoin_conf_with_section() {
        let content = r#"
rpcuser=global_user
rpcpassword=global_pass

[main]
rpcuser=mainnet_user
rpcpassword=mainnet_pass

[test]
rpcuser=testnet_user
rpcpassword=testnet_pass
rpcport=18332

[regtest]
rpcuser=regtest_user
"#;
        // Test mainnet
        let parsed = parse_bitcoin_conf(content, "bitcoin");
        assert_eq!(parsed.get("rpcuser"), Some(&"mainnet_user".to_string()));
        assert_eq!(parsed.get("rpcpassword"), Some(&"mainnet_pass".to_string()));

        // Test testnet
        let parsed = parse_bitcoin_conf(content, "testnet");
        assert_eq!(parsed.get("rpcuser"), Some(&"testnet_user".to_string()));
        assert_eq!(parsed.get("rpcpassword"), Some(&"testnet_pass".to_string()));
        assert_eq!(parsed.get("rpcport"), Some(&"18332".to_string()));

        // Test regtest - section only overrides rpcuser, rpcpassword comes from global
        let parsed = parse_bitcoin_conf(content, "regtest");
        assert_eq!(parsed.get("rpcuser"), Some(&"regtest_user".to_string()));
        assert_eq!(parsed.get("rpcpassword"), Some(&"global_pass".to_string()));
    }

    #[test]
    fn test_parse_bitcoin_conf_comments_and_whitespace() {
        let content = r#"
# This is a comment
  rpcuser = spaced_user
rpcpassword=pass  # inline comment not supported, this is the password

# Another comment
[main]
  rpcport = 8332
"#;
        let parsed = parse_bitcoin_conf(content, "bitcoin");
        assert_eq!(parsed.get("rpcuser"), Some(&"spaced_user".to_string()));
        // Note: inline comments aren't supported by bitcoin.conf, but we trim
        assert_eq!(
            parsed.get("rpcpassword"),
            Some(&"pass  # inline comment not supported, this is the password".to_string())
        );
        assert_eq!(parsed.get("rpcport"), Some(&"8332".to_string()));
    }
}
