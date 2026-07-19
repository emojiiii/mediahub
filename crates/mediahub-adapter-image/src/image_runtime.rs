// Bounded blocking image execution runtime.

async fn run_blocking<T>(
    work: impl FnOnce() -> Result<T, ImageProcessorError> + Send + 'static,
) -> Result<T, ImageProcessorError>
where
    T: Send + 'static,
{
    run_blocking_with_timeout(PROCESSING_TIMEOUT, work).await
}

async fn run_blocking_with_timeout<T>(
    timeout: Duration,
    work: impl FnOnce() -> Result<T, ImageProcessorError> + Send + 'static,
) -> Result<T, ImageProcessorError>
where
    T: Send + 'static,
{
    run_blocking_with_slots(timeout, Arc::clone(&BLOCKING_IMAGE_SLOTS), work).await
}

async fn run_blocking_with_slots<T>(
    timeout: Duration,
    slots: Arc<tokio::sync::Semaphore>,
    work: impl FnOnce() -> Result<T, ImageProcessorError> + Send + 'static,
) -> Result<T, ImageProcessorError>
where
    T: Send + 'static,
{
    let processing = async move {
        let permit = slots
            .acquire_owned()
            .await
            .map_err(|_| ImageProcessorError::ProcessingFailed)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work()
        })
        .await
        .map_err(|_| ImageProcessorError::ProcessingFailed)?
    };
    tokio::time::timeout(timeout, processing)
        .await
        .map_err(|_| ImageProcessorError::ProcessingFailed)?
}

