// Share connection pools across consecutive tool rounds and local voice requests.
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::time::Duration;

pub(crate) const REMOTE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct RemoteState { client: Option<reqwest::blocking::Client>, generation: u64 }

#[derive(Default)]
struct RemotePool { state: Mutex<RemoteState> }

pub(crate) struct RemoteClient { pub client: reqwest::blocking::Client, generation: u64 }

impl RemotePool {
    fn get(&self) -> Result<RemoteClient, String> {
        let mut state = self.state.lock();
        if state.client.is_none() {
            state.client = Some(reqwest::blocking::Client::builder()
                .connect_timeout(REMOTE_CONNECT_TIMEOUT)
                .pool_idle_timeout(Duration::from_secs(30))
                .retry(reqwest::retry::never())
                .build().map_err(|_| "не удалось подготовить соединение".to_string())?);
        }
        Ok(RemoteClient { client: state.client.as_ref().unwrap().clone(), generation: state.generation })
    }

    fn invalidate(&self, client: &RemoteClient) {
        let mut state = self.state.lock();
        // A cancelled older request must not discard a newer, recovered connection pool.
        if state.generation == client.generation {
            state.client = None;
            state.generation = state.generation.wrapping_add(1);
        }
    }
}

static REMOTE: Lazy<RemotePool> = Lazy::new(RemotePool::default);

pub(crate) fn remote_client() -> Result<RemoteClient, String> { REMOTE.get() }
pub(crate) fn reset_remote(client: &RemoteClient) { REMOTE.invalidate(client); }

pub fn client() -> Result<&'static reqwest::blocking::Client, String> {
    static CLIENT: Lazy<Result<reqwest::blocking::Client, String>> = Lazy::new(|| {
        reqwest::blocking::Client::builder().build().map_err(|e| e.to_string())
    });
    CLIENT.as_ref().map_err(Clone::clone)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_transport_failure_rebuilds_the_pool_and_late_failures_cannot_clear_it() {
        let pool = RemotePool::default();
        let first = pool.get().unwrap();
        let concurrent = pool.get().unwrap();
        assert_eq!(first.generation, concurrent.generation);
        pool.invalidate(&first);
        let recovered = pool.get().unwrap();
        assert_ne!(first.generation, recovered.generation);
        pool.invalidate(&concurrent);
        assert_eq!(pool.get().unwrap().generation, recovered.generation);
    }
}
