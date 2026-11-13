//! S3-compatible object storage. Binary stays out of PostgreSQL.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use futures_util::stream::{BoxStream, StreamExt as _};
use object_store::aws::{AmazonS3, AmazonS3Builder};
use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::signer::Signer as _;
use object_store::{
    ClientOptions, GetOptions, GetRange, ObjectStoreExt as _, PutPayload, RetryConfig,
};

#[derive(Clone)]
pub struct ObjectStore {
    inner: Arc<dyn object_store::ObjectStore>,
    /// The S3 client again, for the bucket request `object_store` does not make. Absent for the
    /// in-memory store used by tests.
    s3: Option<Arc<AmazonS3>>,
    /// Test hook: the next this many `delete` calls fail before they touch the store. Shared by
    /// clones. Production leaves it at zero.
    delete_failures: Arc<AtomicU32>,
}

impl ObjectStore {
    /// The object store the environment describes. `PENFII_S3_ENDPOINT`, `_BUCKET`, `_ACCESS_KEY` and `_SECRET_KEY`
    /// are set together or not at all: none is "no object storage" (`Ok(None)`), some is an error that names what
    /// is missing. `PENFII_S3_REGION` is optional.
    pub fn from_env() -> Result<Option<Self>, String> {
        Self::from_lookup(&crate::config::process_env)
    }

    pub fn from_lookup(lookup: crate::config::Lookup<'_>) -> Result<Option<Self>, String> {
        let Some(values) = crate::config::group(
            lookup,
            "object storage",
            &[
                "PENFII_S3_ENDPOINT",
                "PENFII_S3_BUCKET",
                "PENFII_S3_ACCESS_KEY",
                "PENFII_S3_SECRET_KEY",
            ],
        )?
        else {
            return Ok(None);
        };
        let [endpoint, bucket, access_key, secret_key]: [String; 4] = values
            .try_into()
            .map_err(|_| "object storage: four settings are needed".to_owned())?;
        let region =
            crate::config::get(lookup, "PENFII_S3_REGION").unwrap_or_else(|| "us-east-1".into());
        let endpoint = endpoint.trim_end_matches('/').to_string();
        // Path-style requests (`{endpoint}/{bucket}/{key}`), which MinIO needs. The client default
        // timeouts stay (5 s connect, 30 s per request); retries are capped so a stuck store fails
        // the request within seconds instead of the default three minutes.
        let s3 = AmazonS3Builder::new()
            .with_endpoint(&endpoint)
            .with_bucket_name(&bucket)
            .with_region(&region)
            .with_access_key_id(&access_key)
            .with_secret_access_key(&secret_key)
            .with_virtual_hosted_style_request(false)
            .with_client_options(
                ClientOptions::new().with_allow_http(endpoint.starts_with("http://")),
            )
            .with_retry(RetryConfig {
                max_retries: 2,
                retry_timeout: Duration::from_secs(10),
                ..Default::default()
            })
            .build();
        let s3 = match s3 {
            Ok(s3) => Arc::new(s3),
            Err(error) => return Err(format!("object storage is not usable: {error}")),
        };
        Ok(Some(Self {
            inner: s3.clone(),
            s3: Some(s3),
            delete_failures: Arc::new(AtomicU32::new(0)),
        }))
    }

    /// An object store that keeps bytes in memory. Tests use it so file sharing can be checked end
    /// to end without MinIO; the product never builds one.
    pub fn in_memory() -> Self {
        Self {
            inner: Arc::new(InMemory::new()),
            s3: None,
            delete_failures: Arc::new(AtomicU32::new(0)),
        }
    }

    /// The in-memory store with every write held for `delay`, so a test can have two requests
    /// of one upload inside the write at the same time. The product never builds one.
    pub fn in_memory_slow_writes(delay: Duration) -> Self {
        let config = object_store::throttle::ThrottleConfig {
            wait_put_per_call: delay,
            ..Default::default()
        };
        Self {
            inner: Arc::new(object_store::throttle::ThrottledStore::new(
                InMemory::new(),
                config,
            )),
            s3: None,
            delete_failures: Arc::new(AtomicU32::new(0)),
        }
    }

    /// The next `count` calls to [`delete`] fail without touching the store. Tests use it. Clones
    /// share the counter.
    pub fn fail_next_deletes(&self, count: u32) {
        self.delete_failures.store(count, Ordering::SeqCst);
    }

    pub async fn ensure_bucket(&self) -> Result<(), String> {
        let Some(s3) = &self.s3 else {
            return Ok(());
        };
        // A presigned `PUT {endpoint}/{bucket}/` is CreateBucket.
        let url = s3
            .signed_url(
                reqwest::Method::PUT,
                &Path::default(),
                Duration::from_secs(60),
            )
            .await
            .map_err(|error| error.to_string())?;
        let status = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| error.to_string())?
            .put(url)
            .send()
            .await
            .map_err(|error| error.to_string())?
            .status()
            .as_u16();
        if status == 200 || status == 409 {
            return Ok(());
        }
        Err(format!("bucket request failed: {status}"))
    }

    /// Writes an object. The payload is the caller's own buffers (`Bytes`, or a list of chunks), written as
    /// they are: no copy of the content is made here.
    pub async fn put(&self, key: &str, payload: impl Into<PutPayload>) -> Result<(), String> {
        self.inner
            .put(&Path::from(key), payload.into())
            .await
            .map(|_| ())
            .map_err(|error| format!("object write failed: {error}"))
    }

    pub async fn get(&self, key: &str) -> Result<Vec<u8>, String> {
        let read = async { self.inner.get(&Path::from(key)).await?.bytes().await };
        read.await
            .map(|bytes| bytes.to_vec())
            .map_err(|error| format!("object read failed: {error}"))
    }

    /// Opens an object for streaming. `range` is a half-open byte range inside the object; without
    /// it the whole object is served. Nothing but the requested bytes is read into memory at once.
    pub async fn open(
        &self,
        key: &str,
        range: Option<std::ops::Range<u64>>,
    ) -> Result<Opened, String> {
        let options = GetOptions {
            range: range.map(GetRange::Bounded),
            ..GetOptions::default()
        };
        let result = self
            .inner
            .get_opts(&Path::from(key), options)
            .await
            .map_err(|error| format!("object read failed: {error}"))?;
        let size = result.meta.size;
        let range = result.range.clone();
        let stream = result
            .into_stream()
            .map(|chunk| chunk.map_err(|error| std::io::Error::other(error.to_string())))
            .boxed();
        Ok(Opened {
            size,
            range,
            stream,
        })
    }

    /// The size of a stored object, without reading it.
    pub async fn size(&self, key: &str) -> Result<u64, String> {
        self.inner
            .head(&Path::from(key))
            .await
            .map(|meta| meta.size)
            .map_err(|error| format!("object read failed: {error}"))
    }

    /// A missing object counts as deleted.
    pub async fn delete(&self, key: &str) -> Result<(), String> {
        if self.delete_failures.load(Ordering::SeqCst) > 0 {
            self.delete_failures.fetch_sub(1, Ordering::SeqCst);
            return Err("object delete failed: injected".into());
        }
        match self.inner.delete(&Path::from(key)).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(error) => Err(format!("object delete failed: {error}")),
        }
    }
}

/// An object being read: its full size, the byte range the stream carries, and the stream.
pub struct Opened {
    pub size: u64,
    pub range: std::ops::Range<u64>,
    pub stream: BoxStream<'static, Result<bytes::Bytes, std::io::Error>>,
}
