// Share connection pools across consecutive tool rounds and local voice requests.
use once_cell::sync::Lazy;

pub fn client() -> Result<&'static reqwest::blocking::Client, String> {
    static CLIENT: Lazy<Result<reqwest::blocking::Client, String>> = Lazy::new(|| {
        reqwest::blocking::Client::builder().build().map_err(|e| e.to_string())
    });
    CLIENT.as_ref().map_err(Clone::clone)
}
