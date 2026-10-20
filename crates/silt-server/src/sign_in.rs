//! Signing in through GitHub or an OIDC provider (ADR 0013): the
//! authorization code flow with `state` and PKCE, reading who signed in from
//! the provider's user endpoint.

use serde::Deserialize;
use serde_json::Value;
use sha2::Digest;

use crate::config::{Config, ConfigError, secret};

#[derive(Debug, thiserror::Error)]
pub enum SignInError {
    #[error("{0} sign-in is not configured")]
    NotConfigured(String),
    #[error("the provider could not be reached: {0}")]
    Http(#[from] reqwest::Error),
    #[error("the provider answered unexpectedly: {0}")]
    Provider(String),
    #[error(transparent)]
    Config(#[from] ConfigError),
}

/// Who signed in, as the provider says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Identity {
    /// `github` or `oidc:<issuer>`.
    pub(crate) provider: String,
    pub(crate) subject: String,
    /// A name to base a new account's name on.
    pub(crate) login: String,
    pub(crate) display_name: Option<String>,
    /// Only when the provider says it is verified.
    pub(crate) email: Option<String>,
}

pub(crate) struct Endpoints {
    pub(crate) authorize: String,
    pub(crate) token: String,
    pub(crate) userinfo: String,
    pub(crate) client_id: String,
    pub(crate) client_secret: String,
    pub(crate) scopes: String,
    pub(crate) provider: String,
}

#[derive(Deserialize)]
struct Discovery {
    authorization_endpoint: String,
    token_endpoint: String,
    userinfo_endpoint: String,
}

/// The endpoints of `provider` (`github` or `oidc`).
pub(crate) async fn endpoints(config: &Config, http: &reqwest::Client, provider: &str) -> Result<Endpoints, SignInError> {
    match provider {
        "github" => {
            let github = config.auth.github.as_ref().ok_or_else(|| SignInError::NotConfigured("GitHub".into()))?;
            let web = github.web_url.trim_end_matches('/');
            Ok(Endpoints {
                authorize: format!("{web}/login/oauth/authorize"),
                token: format!("{web}/login/oauth/access_token"),
                userinfo: github.api_url.trim_end_matches('/').to_owned(),
                client_id: github.client_id.clone(),
                client_secret: secret(github.client_secret.as_ref(), github.client_secret_file.as_ref())?,
                scopes: "read:user user:email".into(),
                provider: "github".into(),
            })
        }
        "oidc" => {
            let oidc = config.auth.oidc.as_ref().ok_or_else(|| SignInError::NotConfigured("OIDC".into()))?;
            let issuer = oidc.issuer.trim_end_matches('/');
            let discovery: Discovery = http
                .get(format!("{issuer}/.well-known/openid-configuration"))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            Ok(Endpoints {
                authorize: discovery.authorization_endpoint,
                token: discovery.token_endpoint,
                userinfo: discovery.userinfo_endpoint,
                client_id: oidc.client_id.clone(),
                client_secret: secret(oidc.client_secret.as_ref(), oidc.client_secret_file.as_ref())?,
                scopes: oidc.scopes.clone(),
                provider: format!("oidc:{issuer}"),
            })
        }
        other => Err(SignInError::NotConfigured(other.to_owned())),
    }
}

/// The URL that sends the browser to the provider.
pub(crate) fn authorize_url(endpoints: &Endpoints, redirect_uri: &str, state: &str, verifier: &str) -> String {
    let challenge = base64url(&sha2::Sha256::digest(verifier.as_bytes()));
    let query = [
        ("response_type", "code"),
        ("client_id", &endpoints.client_id),
        ("redirect_uri", redirect_uri),
        ("scope", &endpoints.scopes),
        ("state", state),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
    ];
    let query: Vec<String> = query.iter().map(|(key, value)| format!("{key}={}", encode(value))).collect();
    let separator = if endpoints.authorize.contains('?') { '&' } else { '?' };
    format!("{}{separator}{}", endpoints.authorize, query.join("&"))
}

/// Exchanges the code for an access token and reads who signed in.
pub(crate) async fn finish(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    redirect_uri: &str,
    code: &str,
    verifier: &str,
) -> Result<Identity, SignInError> {
    let token: Value = http
        .post(&endpoints.token)
        .header("accept", "application/json")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", &endpoints.client_id),
            ("client_secret", &endpoints.client_secret),
            ("code_verifier", verifier),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let access = token
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| SignInError::Provider(format!("no access token: {token}")))?;
    let get = |url: String| {
        http.get(url)
            .bearer_auth(access)
            .header("accept", "application/json")
            .header("user-agent", "silt-server")
            .send()
    };
    if endpoints.provider == "github" {
        let user: Value = get(format!("{}/user", endpoints.userinfo)).await?.error_for_status()?.json().await?;
        let emails: Vec<Value> = match get(format!("{}/user/emails", endpoints.userinfo)).await {
            Ok(response) if response.status().is_success() => response.json().await.unwrap_or_default(),
            _ => Vec::new(),
        };
        let email = emails
            .iter()
            .find(|email| email["primary"] == true && email["verified"] == true)
            .and_then(|email| email["email"].as_str())
            .map(str::to_owned);
        let subject = user
            .get("id")
            .map(|id| id.as_i64().map_or_else(|| id.to_string(), |id| id.to_string()))
            .ok_or_else(|| SignInError::Provider("GitHub sent no user id".into()))?;
        return Ok(Identity {
            provider: "github".into(),
            subject,
            login: user["login"].as_str().unwrap_or("user").to_owned(),
            display_name: user["name"].as_str().map(str::to_owned),
            email,
        });
    }
    let claims: Value = get(endpoints.userinfo.clone()).await?.error_for_status()?.json().await?;
    let subject = claims["sub"]
        .as_str()
        .ok_or_else(|| SignInError::Provider("the userinfo has no subject".into()))?
        .to_owned();
    let email = claims["email"]
        .as_str()
        .filter(|_| claims["email_verified"] == true)
        .map(str::to_owned);
    let login = claims["preferred_username"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| email.as_deref().and_then(|email| email.split('@').next()).map(str::to_owned))
        .unwrap_or_else(|| subject.clone());
    Ok(Identity {
        provider: endpoints.provider.clone(),
        subject,
        login,
        display_name: claims["name"].as_str().map(str::to_owned),
        email,
    })
}

pub(crate) fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let mut buffer = [0u8; 3];
        buffer[..chunk.len()].copy_from_slice(chunk);
        let triple = (u32::from(buffer[0]) << 16) | (u32::from(buffer[1]) << 8) | u32::from(buffer[2]);
        for index in 0..=chunk.len() {
            out.push(char::from(ALPHABET[((triple >> (18 - 6 * index)) & 0x3f) as usize]));
        }
    }
    out
}

/// Percent-encodes everything but unreserved characters.
pub(crate) fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => char::from(byte).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_the_rfc_example() {
        let challenge = base64url(&sha2::Sha256::digest(b"dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"));
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(encode("a b/c"), "a%20b%2Fc");
    }
}
