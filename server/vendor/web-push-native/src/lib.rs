//! RFC 8030 / RFC 8291 Web Push encryption and request building (see README.md for what differs from upstream).

#[cfg(test)]
mod tests;

use std::time::Duration;

use aes_gcm::aead::generic_array::{typenum::U16, GenericArray};
use hkdf::Hkdf;
use http::{self, header, Request, Uri};
pub use p256;
use p256::elliptic_curve::sec1::ToSec1Point;
use sha2::Sha256;

/// Error type for HTTP push failure modes
#[derive(Debug)]
pub enum Error {
    /// Internal ECE error
    ECE(ece_native::Error),
    /// Internal error coming from an http auth provider
    Extension(Box<dyn std::error::Error + Send + Sync + 'static>),
    /// The system could not give random bytes
    Random,
}

impl std::error::Error for Error {}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ECE(ece) => write!(f, "ECE: {}", ece),
            Error::Extension(ext) => write!(f, "Extension: {}", ext),
            Error::Random => write!(f, "no random bytes"),
        }
    }
}

/// HTTP push authentication secret
pub type Auth = GenericArray<u8, U16>;

/// Reusable builder for HTTP push requests
#[derive(Clone, Debug)]
pub struct WebPushBuilder {
    endpoint: Uri,
    valid_duration: Duration,
    ua_public: p256::PublicKey,
    ua_auth: Auth,
}

impl WebPushBuilder {
    /// Creates a new [`WebPushBuilder`] factory for HTTP push requests.
    ///
    /// Requests generated using this factory will have a valid  duration of 12
    /// hours and no VAPID signature.
    ///
    /// Most providers accepting HTTP push requests will require a valid VAPID
    /// signature, so you will most likely want to add one using
    /// [`WebPushBuilder::with_vapid`].
    pub fn new(endpoint: Uri, ua_public: p256::PublicKey, ua_auth: Auth) -> Self {
        Self {
            endpoint,
            ua_public,
            ua_auth,
            valid_duration: Duration::from_secs(12 * 60 * 60),
        }
    }

    /// Sets the valid duration for generated HTTP push requests.
    pub fn with_valid_duration(self, valid_duration: Duration) -> Self {
        Self {
            valid_duration,
            ..self
        }
    }
}

impl WebPushBuilder {
    /// Generates a new HTTP push request according to the
    /// specifications of the builder.
    pub fn build<T: Into<Vec<u8>>>(&self, body: T) -> Result<Request<Vec<u8>>, Error> {
        let body = body.into();

        let payload = encrypt(body, &self.ua_public, &self.ua_auth)?;
        let builder = Request::builder()
            .uri(self.endpoint.clone())
            .method(http::method::Method::POST)
            .header("TTL", self.valid_duration.as_secs())
            .header(header::CONTENT_ENCODING, "aes128gcm")
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(header::CONTENT_LENGTH, payload.len());

        Ok(builder
            .body(payload)
            .expect("builder arguments are always well-defined"))
    }
}

/// A fresh P-256 private key (drawn again in the rare case the bytes are not a valid scalar).
fn random_secret() -> Result<p256::SecretKey, Error> {
    loop {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| Error::Random)?;
        if let Ok(secret) = p256::SecretKey::from_slice(&bytes) {
            return Ok(secret);
        }
    }
}

/// Lower-level encryption used for HTTP push request content
pub fn encrypt(
    message: Vec<u8>,
    ua_public: &p256::PublicKey,
    ua_auth: &Auth,
) -> Result<Vec<u8>, Error> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|_| Error::Random)?;
    let as_secret = random_secret()?;
    encrypt_predictably(salt, message, &as_secret, ua_public, ua_auth).map_err(Error::ECE)
}

fn encrypt_predictably(
    salt: [u8; 16],
    message: Vec<u8>,
    as_secret: &p256::SecretKey,
    ua_public: &p256::PublicKey,
    ua_auth: &Auth,
) -> Result<Vec<u8>, ece_native::Error> {
    let as_public = as_secret.public_key();
    let shared = p256::ecdh::diffie_hellman(as_secret.to_nonzero_scalar(), ua_public.as_affine());

    let ikm = compute_ikm(ua_auth, &shared, ua_public, &as_public);
    let keyid = as_public.as_affine().to_sec1_point(false);
    let encrypted_record_length = (message.len() + 17)
        .try_into()
        .map_err(|_| ece_native::Error::RecordLengthInvalid)?;

    ece_native::encrypt(
        ikm,
        salt,
        keyid,
        Some(message).into_iter(),
        encrypted_record_length,
    )
}

/// Lower-level decryption used for HTTP push request content
pub fn decrypt(
    encrypted_message: Vec<u8>,
    as_secret: &p256::SecretKey,
    ua_auth: &Auth,
) -> Result<Vec<u8>, Error> {
    let keyid = view_keyid(&encrypted_message).map_err(Error::ECE)?;
    let ua_public = p256::PublicKey::from_sec1_bytes(keyid)
        .map_err(|_| ece_native::Error::Aes128Gcm)
        .map_err(Error::ECE)?;
    let shared = p256::ecdh::diffie_hellman(as_secret.to_nonzero_scalar(), ua_public.as_affine());

    let ikm = compute_ikm(ua_auth, &shared, &as_secret.public_key(), &ua_public);

    ece_native::decrypt(ikm, encrypted_message).map_err(Error::ECE)
}

fn compute_ikm(
    auth: &Auth,
    shared: &p256::ecdh::SharedSecret,
    ua_public: &p256::PublicKey,
    as_public: &p256::PublicKey,
) -> [u8; 32] {
    let mut info = Vec::new();
    info.extend_from_slice(&b"WebPush: info"[..]);
    info.push(0u8);
    info.extend_from_slice(ua_public.as_affine().to_sec1_point(false).as_bytes());
    info.extend_from_slice(as_public.as_affine().to_sec1_point(false).as_bytes());

    let mut okm = [0u8; 32];
    let hk = Hkdf::<Sha256>::new(Some(auth), shared.raw_secret_bytes().as_ref());
    hk.expand(&info, &mut okm)
        .expect("okm length is always 32 bytes, cannot be too large");

    okm
}

fn view_keyid(encrypted_message: &[u8]) -> Result<&[u8], ece_native::Error> {
    if encrypted_message.len() < 21 {
        return Err(ece_native::Error::HeaderLengthInvalid);
    }

    let idlen: usize = encrypted_message[20].into();
    if encrypted_message[21..].len() < idlen {
        return Err(ece_native::Error::KeyIdLengthInvalid);
    }

    Ok(&encrypted_message[21..21 + idlen])
}
