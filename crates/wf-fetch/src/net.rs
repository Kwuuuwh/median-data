use std::io::Read;
use std::time::Duration;

pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36";

/// Build a blocking ureq agent with a global per-call timeout.
pub fn http_agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(300)))
        .build();
    ureq::Agent::new_with_config(config)
}

/// GET a URL and read the full response body into bytes.
pub fn get_bytes(agent: &ureq::Agent, url: &str) -> anyhow::Result<Vec<u8>> {
    let mut res = agent
        .get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| anyhow::anyhow!("GET {url}: {e}"))?;
    let mut buf = Vec::new();
    res.body_mut()
        .as_reader()
        .read_to_end(&mut buf)
        .map_err(|e| anyhow::anyhow!("read {url}: {e}"))?;
    Ok(buf)
}
