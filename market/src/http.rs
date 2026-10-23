//! HTTPS GET with a time limit: a slow network must not leave a startup hook hanging.

use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(20);
/// herdr's index is about 2 MB today; this leaves room without letting a broken server fill memory.
const MAX_BYTES: u64 = 64 * 1024 * 1024;

pub fn get(url: &str) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).build().into();
    let mut response = agent.get(url).call().map_err(|e| format!("{url}: {e}"))?;
    response.body_mut().with_config().limit(MAX_BYTES).read_to_string().map_err(|e| format!("{url}: {e}"))
}
