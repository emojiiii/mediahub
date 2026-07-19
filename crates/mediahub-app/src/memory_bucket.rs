// In-memory bucket repository.

#[derive(Clone, Default)]
pub struct InMemoryBucketRepository {
    buckets: Arc<Mutex<HashMap<BucketId, Bucket>>>,
}

impl InMemoryBucketRepository {
    #[must_use]
    pub fn with_bucket(bucket: Bucket) -> Self {
        let repository = Self::default();
        repository.insert(bucket);
        repository
    }

    pub fn insert(&self, bucket: Bucket) {
        self.buckets
            .lock()
            .expect("in-memory bucket repository lock")
            .insert(bucket.id(), bucket);
    }
}

#[async_trait]
impl BucketRepository for InMemoryBucketRepository {
    async fn find_by_id(&self, bucket_id: BucketId) -> Result<Option<Bucket>, RepositoryError> {
        Ok(self
            .buckets
            .lock()
            .map_err(|_| {
                RepositoryError::Unavailable("in-memory bucket repository lock poisoned".into())
            })?
            .get(&bucket_id)
            .cloned())
    }
}
