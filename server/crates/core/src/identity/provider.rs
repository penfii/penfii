//! External sign-in providers.
//!
//! `OidcProvider` is one implementation driven by configuration; Google and Microsoft are presets
//! of it and nothing outside the presets names a provider. GitHub has no ID token and is a
//! separate `GithubProvider`. All network access goes through `Transport`, so tests run against a
//! fake that signs real JWTs.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde_json::{Value, json};

use crate::error::CoreError;

const JWKS_MAX_AGE: Duration = Duration::from_secs(3600);
const RESPONSE_LIMIT: usize = 256 * 1024;

fn upstream(message: &str) -> CoreError {
    CoreError::conflict("provider_error", message.to_string())
}

fn invalid_token(message: &str) -> CoreError {
    CoreError::conflict("id_token_invalid", message.to_string())
}

/// The only way providers talk to the network.
pub trait Transport: Send + Sync {
    fn get(&self, url: &str, bearer: Option<&str>) -> BoxFuture<'_, Result<Value, CoreError>>;
    fn post_form(
        &self,
        url: &str,
        form: Vec<(String, String)>,
    ) -> BoxFuture<'_, Result<Value, CoreError>>;
}

/// TLS verified, no redirects, short timeout, bounded response.
pub struct HttpTransport {
    client: reqwest::Client,
}

impl HttpTransport {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(8))
            .user_agent("penfii")
            .build()
            .expect("http client");
        Self { client }
    }

    async fn read(response: reqwest::Response) -> Result<Value, CoreError> {
        let mut response = response;
        if !response.status().is_success() {
            return Err(upstream("provider answered with an error"));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| upstream("provider response failed"))?
        {
            body.extend_from_slice(&chunk);
            if body.len() > RESPONSE_LIMIT {
                return Err(upstream("provider response is too large"));
            }
        }
        serde_json::from_slice(&body).map_err(|_| upstream("provider response is not JSON"))
    }
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for HttpTransport {
    fn get(&self, url: &str, bearer: Option<&str>) -> BoxFuture<'_, Result<Value, CoreError>> {
        let mut request = self.client.get(url).header("accept", "application/json");
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        Box::pin(async move {
            let response = request
                .send()
                .await
                .map_err(|_| upstream("provider is unreachable"))?;
            Self::read(response).await
        })
    }

    fn post_form(
        &self,
        url: &str,
        form: Vec<(String, String)>,
    ) -> BoxFuture<'_, Result<Value, CoreError>> {
        let request = self
            .client
            .post(url)
            .header("accept", "application/json")
            .form(&form);
        Box::pin(async move {
            let response = request
                .send()
                .await
                .map_err(|_| upstream("provider is unreachable"))?;
            Self::read(response).await
        })
    }
}

#[derive(Debug, Clone)]
pub struct ExternalProfile {
    pub issuer: String,
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
    /// The provider can vouch that `email` belongs to this person, which is what accepting an
    /// invitation by external sign-in needs.
    pub email_proves_ownership: bool,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    /// Non-authoritative snapshot (for example Microsoft `tid` / `oid`).
    pub claims: Value,
}

pub struct AuthorizeRequest<'a> {
    pub redirect_uri: &'a str,
    pub state: &'a str,
    pub code_challenge: &'a str,
    pub nonce: &'a str,
    /// Ask the provider for a fresh login (reauthentication).
    pub force_login: bool,
    /// The invitation path also needs the provider's e-mail.
    pub want_email: bool,
}

pub struct CompleteRequest<'a> {
    pub redirect_uri: &'a str,
    pub code: &'a str,
    pub code_verifier: &'a str,
    pub nonce: &'a str,
    pub want_email: bool,
}

pub trait Provider: Send + Sync {
    fn key(&self) -> &str;
    fn kind(&self) -> &str;
    fn display_name(&self) -> &str;
    /// Stored in `auth_providers.issuer` (informational for template issuers).
    fn issuer(&self) -> String;
    fn client_id(&self) -> &str;
    /// Name of the environment variable that holds the secret.
    fn secret_ref(&self) -> &str;
    /// Non-secret configuration for `auth_providers.config`.
    fn config(&self) -> Value;
    /// The operator allows a first sign-in with an identity nobody has linked to create the account
    /// (`PENFII_AUTH_<NAME>_SIGNUP`). Off unless configured, which keeps accounts invitation-only.
    fn signup_allowed(&self) -> bool {
        false
    }
    fn authorize_url(
        &self,
        request: AuthorizeRequest<'_>,
    ) -> BoxFuture<'_, Result<String, CoreError>>;
    fn complete<'a>(
        &'a self,
        request: CompleteRequest<'a>,
    ) -> BoxFuture<'a, Result<ExternalProfile, CoreError>>;
}

#[derive(Clone)]
pub struct Endpoints {
    pub authorization: String,
    pub token: String,
    pub jwks: String,
}

#[derive(Clone)]
pub enum IssuerPolicy {
    /// `iss` must equal this string.
    Fixed(String),
    /// Microsoft multi-tenant: `iss` must be `https://login.microsoftonline.com/{tid}/v2.0` for
    /// the token's own `tid`; `allowed` limits the tenants.
    MicrosoftTenants { allowed: Option<Vec<String>> },
}

#[derive(Clone)]
pub struct OidcConfig {
    pub key: String,
    pub kind: &'static str,
    pub display_name: String,
    pub issuer: IssuerPolicy,
    /// Explicit endpoints, or `None` to discover them from `{issuer}/.well-known/openid-configuration`.
    pub endpoints: Option<Endpoints>,
    pub client_id: String,
    pub secret_ref: String,
    pub client_secret: String,
    pub scopes: String,
    pub subject_claim: String,
    /// Google `hd`: only this hosted domain.
    pub hosted_domain: Option<String>,
    /// Microsoft: the e-mail claim proves ownership only when one tenant is pinned.
    pub email_proof_requires_pinned_tenant: bool,
}

pub struct OidcProvider {
    config: OidcConfig,
    transport: Arc<dyn Transport>,
    endpoints: Mutex<Option<Endpoints>>,
    jwks: Mutex<Option<(Instant, JwkSet)>>,
    signup: bool,
}

impl OidcProvider {
    pub fn new(config: OidcConfig, transport: Arc<dyn Transport>) -> Self {
        let endpoints = Mutex::new(config.endpoints.clone());
        Self {
            config,
            transport,
            endpoints,
            jwks: Mutex::new(None),
            signup: false,
        }
    }

    /// Let a first sign-in create the Penfii account (see `Provider::signup_allowed`).
    pub fn with_signup(mut self, allowed: bool) -> Self {
        self.signup = allowed;
        self
    }

    pub fn google(
        client_id: String,
        secret_ref: String,
        client_secret: String,
        hosted_domain: Option<String>,
        transport: Arc<dyn Transport>,
    ) -> Self {
        Self::new(
            OidcConfig {
                key: "google".into(),
                kind: "google",
                display_name: "Google".into(),
                issuer: IssuerPolicy::Fixed("https://accounts.google.com".into()),
                endpoints: Some(Endpoints {
                    authorization: "https://accounts.google.com/o/oauth2/v2/auth".into(),
                    token: "https://oauth2.googleapis.com/token".into(),
                    jwks: "https://www.googleapis.com/oauth2/v3/certs".into(),
                }),
                client_id,
                secret_ref,
                client_secret,
                scopes: "openid profile email".into(),
                subject_claim: "sub".into(),
                hosted_domain,
                email_proof_requires_pinned_tenant: false,
            },
            transport,
        )
    }

    /// `tenant` is a tenant id / domain to pin, or `common` / `organizations` / `consumers`
    /// together with `allowed` tenant ids.
    pub fn microsoft(
        client_id: String,
        secret_ref: String,
        client_secret: String,
        tenant: String,
        allowed: Option<Vec<String>>,
        transport: Arc<dyn Transport>,
    ) -> Self {
        let multi = matches!(tenant.as_str(), "common" | "organizations" | "consumers");
        let issuer = if multi {
            IssuerPolicy::MicrosoftTenants { allowed }
        } else {
            IssuerPolicy::Fixed(format!("https://login.microsoftonline.com/{tenant}/v2.0"))
        };
        Self::new(
            OidcConfig {
                key: "microsoft".into(),
                kind: "microsoft",
                display_name: "Microsoft".into(),
                issuer,
                endpoints: Some(Endpoints {
                    authorization: format!(
                        "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize"
                    ),
                    token: format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"),
                    jwks: format!("https://login.microsoftonline.com/{tenant}/discovery/v2.0/keys"),
                }),
                client_id,
                secret_ref,
                client_secret,
                scopes: "openid profile email".into(),
                subject_claim: "sub".into(),
                hosted_domain: None,
                email_proof_requires_pinned_tenant: true,
            },
            transport,
        )
    }

    async fn endpoints(&self) -> Result<Endpoints, CoreError> {
        if let Some(found) = self.endpoints.lock().expect("endpoints").clone() {
            return Ok(found);
        }
        let IssuerPolicy::Fixed(issuer) = &self.config.issuer else {
            return Err(upstream("provider has no endpoints"));
        };
        let url = format!(
            "{}/.well-known/openid-configuration",
            issuer.trim_end_matches('/')
        );
        let document = self.transport.get(&url, None).await?;
        // The document must describe the configured issuer (mix-up defence).
        if document["issuer"].as_str() != Some(issuer.as_str()) {
            return Err(upstream("discovery document issuer differs"));
        }
        let pick = |name: &str| {
            document[name]
                .as_str()
                .filter(|value| value.starts_with("https://"))
                .map(str::to_string)
                .ok_or_else(|| upstream("discovery document is incomplete"))
        };
        let found = Endpoints {
            authorization: pick("authorization_endpoint")?,
            token: pick("token_endpoint")?,
            jwks: pick("jwks_uri")?,
        };
        *self.endpoints.lock().expect("endpoints") = Some(found.clone());
        Ok(found)
    }

    async fn key_for(&self, jwks_url: &str, kid: &str) -> Result<DecodingKey, CoreError> {
        for refresh in [false, true] {
            let cached = {
                let guard = self.jwks.lock().expect("jwks");
                match &*guard {
                    Some((at, set)) if !refresh && at.elapsed() < JWKS_MAX_AGE => Some(set.clone()),
                    _ => None,
                }
            };
            let set = match cached {
                Some(set) => set,
                None => {
                    let value = self.transport.get(jwks_url, None).await?;
                    let set: JwkSet = serde_json::from_value(value)
                        .map_err(|_| upstream("provider key set is invalid"))?;
                    *self.jwks.lock().expect("jwks") = Some((Instant::now(), set.clone()));
                    set
                }
            };
            if let Some(jwk) = set.find(kid) {
                return DecodingKey::from_jwk(jwk).map_err(|_| invalid_token("unusable key"));
            }
            // Unknown kid on a fresh set: nothing more to try.
            if refresh {
                break;
            }
        }
        Err(invalid_token("unknown signing key"))
    }

    /// Signature, `iss`, `aud`, `exp` / `nbf`, `nonce`, `azp`.
    async fn verify(
        &self,
        id_token: &str,
        nonce: &str,
        jwks_url: &str,
    ) -> Result<Value, CoreError> {
        let header = decode_header(id_token).map_err(|_| invalid_token("malformed token"))?;
        if header.alg != Algorithm::RS256 {
            return Err(invalid_token("unsupported algorithm"));
        }
        let kid = header
            .kid
            .ok_or_else(|| invalid_token("token has no key id"))?;
        let key = self.key_for(jwks_url, &kid).await?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[self.config.client_id.as_str()]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.validate_nbf = true;
        validation.leeway = 60;
        if let IssuerPolicy::Fixed(issuer) = &self.config.issuer {
            validation.set_issuer(&[issuer.as_str()]);
        }
        let claims = decode::<Value>(id_token, &key, &validation)
            .map_err(|_| invalid_token("token failed verification"))?
            .claims;
        if let IssuerPolicy::MicrosoftTenants { allowed } = &self.config.issuer {
            let tid = claims["tid"]
                .as_str()
                .ok_or_else(|| invalid_token("token has no tenant"))?;
            let expected = format!("https://login.microsoftonline.com/{tid}/v2.0");
            if claims["iss"].as_str() != Some(expected.as_str()) {
                return Err(invalid_token("issuer does not match tenant"));
            }
            if let Some(allowed) = allowed
                && !allowed.iter().any(|tenant| tenant == tid)
            {
                return Err(invalid_token("tenant is not allowed"));
            }
        }
        if claims["nonce"].as_str() != Some(nonce) {
            return Err(invalid_token("nonce mismatch"));
        }
        if let Some(azp) = claims["azp"].as_str()
            && azp != self.config.client_id
        {
            return Err(invalid_token("authorized party mismatch"));
        }
        if let Some(domain) = &self.config.hosted_domain
            && claims["hd"].as_str() != Some(domain.as_str())
        {
            return Err(invalid_token("hosted domain is not allowed"));
        }
        Ok(claims)
    }

    fn profile(&self, claims: Value) -> Result<ExternalProfile, CoreError> {
        let subject = claims[self.config.subject_claim.as_str()]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid_token("token has no subject"))?
            .to_string();
        let issuer = claims["iss"].as_str().unwrap_or_default().to_string();
        let email = claims["email"].as_str().map(str::to_string);
        let flagged = match &claims["email_verified"] {
            Value::Bool(value) => *value,
            Value::String(value) => value == "true",
            _ => false,
        };
        let (email_verified, proves) = if self.config.email_proof_requires_pinned_tenant {
            // Microsoft: an e-mail claim is not proof. The operator vouches for a pinned tenant.
            let pinned = matches!(self.config.issuer, IssuerPolicy::Fixed(_));
            (false, pinned && email.is_some())
        } else {
            (flagged, flagged && email.is_some())
        };
        let mut snapshot = json!({});
        for name in ["tid", "oid", "hd", "preferred_username"] {
            if let Some(value) = claims.get(name) {
                snapshot[name] = value.clone();
            }
        }
        Ok(ExternalProfile {
            issuer,
            subject,
            email,
            email_verified,
            email_proves_ownership: proves,
            display_name: claims["name"].as_str().map(str::to_string),
            avatar_url: claims["picture"].as_str().map(str::to_string),
            claims: snapshot,
        })
    }
}

impl Provider for OidcProvider {
    fn signup_allowed(&self) -> bool {
        self.signup
    }
    fn key(&self) -> &str {
        &self.config.key
    }
    fn kind(&self) -> &str {
        self.config.kind
    }
    fn display_name(&self) -> &str {
        &self.config.display_name
    }
    fn issuer(&self) -> String {
        match &self.config.issuer {
            IssuerPolicy::Fixed(issuer) => issuer.clone(),
            IssuerPolicy::MicrosoftTenants { .. } => {
                "https://login.microsoftonline.com/{tenantid}/v2.0".into()
            }
        }
    }
    fn client_id(&self) -> &str {
        &self.config.client_id
    }
    fn secret_ref(&self) -> &str {
        &self.config.secret_ref
    }
    fn config(&self) -> Value {
        json!({
            "scopes": self.config.scopes,
            "subjectClaim": self.config.subject_claim,
            "hostedDomain": self.config.hosted_domain,
        })
    }

    fn authorize_url(
        &self,
        request: AuthorizeRequest<'_>,
    ) -> BoxFuture<'_, Result<String, CoreError>> {
        let redirect_uri = request.redirect_uri.to_string();
        let state = request.state.to_string();
        let challenge = request.code_challenge.to_string();
        let nonce = request.nonce.to_string();
        let force = request.force_login;
        Box::pin(async move {
            let endpoints = self.endpoints().await?;
            let mut url = url::Url::parse(&endpoints.authorization)
                .map_err(|_| upstream("authorization endpoint is invalid"))?;
            {
                let mut query = url.query_pairs_mut();
                query
                    .append_pair("response_type", "code")
                    .append_pair("client_id", &self.config.client_id)
                    .append_pair("redirect_uri", &redirect_uri)
                    .append_pair("scope", &self.config.scopes)
                    .append_pair("state", &state)
                    .append_pair("nonce", &nonce)
                    .append_pair("code_challenge", &challenge)
                    .append_pair("code_challenge_method", "S256");
                if force {
                    // `max_age=0` is the OIDC way; `prompt=login` is understood by the same providers.
                    query
                        .append_pair("prompt", "login")
                        .append_pair("max_age", "0");
                }
            }
            Ok(url.to_string())
        })
    }

    fn complete<'a>(
        &'a self,
        request: CompleteRequest<'a>,
    ) -> BoxFuture<'a, Result<ExternalProfile, CoreError>> {
        Box::pin(async move {
            let endpoints = self.endpoints().await?;
            let response = self
                .transport
                .post_form(
                    &endpoints.token,
                    vec![
                        ("grant_type".into(), "authorization_code".into()),
                        ("code".into(), request.code.into()),
                        ("redirect_uri".into(), request.redirect_uri.into()),
                        ("client_id".into(), self.config.client_id.clone()),
                        ("client_secret".into(), self.config.client_secret.clone()),
                        ("code_verifier".into(), request.code_verifier.into()),
                    ],
                )
                .await?;
            let id_token = response["id_token"]
                .as_str()
                .ok_or_else(|| upstream("provider returned no ID token"))?;
            let claims = self
                .verify(id_token, request.nonce, &endpoints.jwks)
                .await?;
            // The provider access token is dropped here.
            self.profile(claims)
        })
    }
}

/// GitHub: OAuth 2.0 without an ID token. Identity is the numeric user id from `GET /user`.
pub struct GithubProvider {
    client_id: String,
    secret_ref: String,
    client_secret: String,
    transport: Arc<dyn Transport>,
}

impl GithubProvider {
    pub fn new(
        client_id: String,
        secret_ref: String,
        client_secret: String,
        transport: Arc<dyn Transport>,
    ) -> Self {
        Self {
            client_id,
            secret_ref,
            client_secret,
            transport,
        }
    }
}

impl Provider for GithubProvider {
    fn key(&self) -> &str {
        "github"
    }
    fn kind(&self) -> &str {
        "github"
    }
    fn display_name(&self) -> &str {
        "GitHub"
    }
    fn issuer(&self) -> String {
        "https://github.com".into()
    }
    fn client_id(&self) -> &str {
        &self.client_id
    }
    fn secret_ref(&self) -> &str {
        &self.secret_ref
    }
    fn config(&self) -> Value {
        json!({})
    }

    fn authorize_url(
        &self,
        request: AuthorizeRequest<'_>,
    ) -> BoxFuture<'_, Result<String, CoreError>> {
        let url = (|| {
            let mut url = url::Url::parse("https://github.com/login/oauth/authorize")
                .map_err(|_| upstream("authorization endpoint is invalid"))?;
            {
                let mut query = url.query_pairs_mut();
                query
                    .append_pair("client_id", &self.client_id)
                    .append_pair("redirect_uri", request.redirect_uri)
                    .append_pair("state", request.state)
                    .append_pair("code_challenge", request.code_challenge)
                    .append_pair("code_challenge_method", "S256")
                    .append_pair("allow_signup", "false");
                if request.want_email {
                    query.append_pair("scope", "user:email");
                }
            }
            Ok(url.to_string())
        })();
        Box::pin(async move { url })
    }

    fn complete<'a>(
        &'a self,
        request: CompleteRequest<'a>,
    ) -> BoxFuture<'a, Result<ExternalProfile, CoreError>> {
        Box::pin(async move {
            let response = self
                .transport
                .post_form(
                    "https://github.com/login/oauth/access_token",
                    vec![
                        ("client_id".into(), self.client_id.clone()),
                        ("client_secret".into(), self.client_secret.clone()),
                        ("code".into(), request.code.into()),
                        ("redirect_uri".into(), request.redirect_uri.into()),
                        ("code_verifier".into(), request.code_verifier.into()),
                    ],
                )
                .await?;
            let token = response["access_token"]
                .as_str()
                .ok_or_else(|| upstream("provider returned no access token"))?
                .to_string();
            let user = self
                .transport
                .get("https://api.github.com/user", Some(&token))
                .await?;
            let id = user["id"]
                .as_u64()
                .ok_or_else(|| upstream("provider returned no user id"))?;
            let mut email = None;
            let mut verified = false;
            if request.want_email {
                let list = self
                    .transport
                    .get("https://api.github.com/user/emails", Some(&token))
                    .await?;
                if let Some(primary) = list.as_array().and_then(|items| {
                    items
                        .iter()
                        .find(|item| item["primary"] == true && item["verified"] == true)
                }) {
                    email = primary["email"].as_str().map(str::to_string);
                    verified = email.is_some();
                }
            }
            // The token is dropped when this function returns.
            Ok(ExternalProfile {
                issuer: "https://github.com".into(),
                subject: id.to_string(),
                email,
                email_verified: verified,
                email_proves_ownership: verified,
                display_name: user["name"]
                    .as_str()
                    .or_else(|| user["login"].as_str())
                    .map(str::to_string),
                avatar_url: user["avatar_url"].as_str().map(str::to_string),
                claims: json!({"login": user["login"]}),
            })
        })
    }
}

/// The providers an instance offers. Built from the environment at startup; tests build their own.
#[derive(Clone, Default)]
pub struct AuthProviders {
    pub public_url: Option<String>,
    providers: HashMap<String, Arc<dyn Provider>>,
}

impl AuthProviders {
    pub fn new(public_url: Option<String>) -> Self {
        Self {
            public_url: public_url.map(|url| url.trim_end_matches('/').to_string()),
            providers: HashMap::new(),
        }
    }

    pub fn add(&mut self, provider: Arc<dyn Provider>) {
        self.providers.insert(provider.key().to_string(), provider);
    }

    pub fn get(&self, key: &str) -> Option<&Arc<dyn Provider>> {
        self.providers.get(key)
    }

    pub fn list(&self) -> Vec<&Arc<dyn Provider>> {
        let mut all: Vec<_> = self.providers.values().collect();
        all.sort_by(|a, b| a.key().cmp(b.key()));
        all
    }

    pub fn redirect_uri(&self, key: &str) -> Option<String> {
        self.public_url
            .as_ref()
            .map(|base| format!("{base}/v1/auth/{key}/callback"))
    }

    /// Provider variables are `PENFII_AUTH_<NAME>_CLIENT_ID` and
    /// `PENFII_AUTH_<NAME>_CLIENT_SECRET`; a provider with an incomplete set is not offered.
    pub fn from_env(transport: Arc<dyn Transport>) -> Self {
        let env = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let flag =
            |name: &str| env(name).is_some_and(|value| matches!(value.as_str(), "1" | "true"));
        let mut set = Self::new(env("PENFII_PUBLIC_URL"));
        let pair = |name: &str| {
            let id = format!("PENFII_AUTH_{name}_CLIENT_ID");
            let secret = format!("PENFII_AUTH_{name}_CLIENT_SECRET");
            Some((env(&id)?, secret.clone(), env(&secret)?))
        };
        if let Some((id, secret_ref, secret)) = pair("GOOGLE") {
            set.add(Arc::new(
                OidcProvider::google(
                    id,
                    secret_ref,
                    secret,
                    env("PENFII_AUTH_GOOGLE_HOSTED_DOMAIN"),
                    transport.clone(),
                )
                .with_signup(flag("PENFII_AUTH_GOOGLE_SIGNUP")),
            ));
        }
        if let Some((id, secret_ref, secret)) = pair("MICROSOFT") {
            let tenant =
                env("PENFII_AUTH_MICROSOFT_TENANT").unwrap_or_else(|| "organizations".into());
            let allowed = env("PENFII_AUTH_MICROSOFT_ALLOWED_TENANTS").map(|list| {
                list.split(',')
                    .map(|tenant| tenant.trim().to_string())
                    .filter(|tenant| !tenant.is_empty())
                    .collect::<Vec<_>>()
            });
            let multi = matches!(tenant.as_str(), "common" | "organizations" | "consumers");
            // An unpinned tenant endpoint is only usable with an operator tenant allow-list.
            if !multi || allowed.as_ref().is_some_and(|list| !list.is_empty()) {
                set.add(Arc::new(
                    OidcProvider::microsoft(
                        id,
                        secret_ref,
                        secret,
                        tenant,
                        allowed,
                        transport.clone(),
                    )
                    .with_signup(flag("PENFII_AUTH_MICROSOFT_SIGNUP")),
                ));
            } else {
                tracing::warn!(
                    "Microsoft sign-in is off: set PENFII_AUTH_MICROSOFT_TENANT to one tenant or PENFII_AUTH_MICROSOFT_ALLOWED_TENANTS"
                );
            }
        }
        if let Some((id, secret_ref, secret)) = pair("GITHUB") {
            set.add(Arc::new(GithubProvider::new(
                id,
                secret_ref,
                secret,
                transport.clone(),
            )));
        }
        // Operator-configured generic OIDC providers: PENFII_AUTH_OIDC_PROVIDERS=slug,slug and
        // PENFII_AUTH_OIDC_<SLUG>_{ISSUER,CLIENT_ID,CLIENT_SECRET,NAME}.
        if let Some(list) = env("PENFII_AUTH_OIDC_PROVIDERS") {
            for slug in list
                .split(',')
                .map(str::trim)
                .filter(|slug| !slug.is_empty())
            {
                let upper = slug.to_ascii_uppercase().replace('-', "_");
                let valid = slug
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-');
                let (Some(issuer), Some((id, secret_ref, secret)), true) = (
                    env(&format!("PENFII_AUTH_OIDC_{upper}_ISSUER")),
                    pair(&format!("OIDC_{upper}")),
                    valid,
                ) else {
                    tracing::warn!("generic OIDC provider {slug:?} is incomplete and is off");
                    continue;
                };
                if !issuer.starts_with("https://") {
                    tracing::warn!("generic OIDC provider {slug:?} needs an https issuer");
                    continue;
                }
                set.add(Arc::new(
                    OidcProvider::new(
                        OidcConfig {
                            key: format!("oidc:{slug}"),
                            kind: "oidc",
                            display_name: env(&format!("PENFII_AUTH_OIDC_{upper}_NAME"))
                                .unwrap_or_else(|| slug.to_string()),
                            issuer: IssuerPolicy::Fixed(issuer),
                            endpoints: None,
                            client_id: id,
                            secret_ref,
                            client_secret: secret,
                            scopes: "openid profile email".into(),
                            subject_claim: "sub".into(),
                            hosted_domain: None,
                            email_proof_requires_pinned_tenant: false,
                        },
                        transport.clone(),
                    )
                    .with_signup(flag(&format!("PENFII_AUTH_OIDC_{upper}_SIGNUP"))),
                ));
            }
        }
        if flag("PENFII_AUTH_GITHUB_SIGNUP") {
            tracing::warn!(
                "PENFII_AUTH_GITHUB_SIGNUP is ignored: GitHub accounts are created by invitation only"
            );
        }
        set
    }
}
