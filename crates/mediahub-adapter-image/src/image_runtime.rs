// Bounded blocking image execution runtime.

#[cfg(test)]
async fn run_blocking<T>(
    work: impl FnOnce() -> Result<T, ImageProcessorError> + Send + 'static,
) -> Result<T, ImageProcessorError>
where
    T: Send + 'static,
{
    run_blocking_with_timeout(PROCESSING_TIMEOUT, work).await
}

async fn acquire_blocking_slot() -> Result<tokio::sync::OwnedSemaphorePermit, ImageProcessorError> {
    tokio::time::timeout(
        PROCESSING_TIMEOUT,
        Arc::clone(&BLOCKING_IMAGE_SLOTS).acquire_owned(),
    )
    .await
    .map_err(|_| ImageProcessorError::ProcessingFailed)?
    .map_err(|_| ImageProcessorError::ProcessingFailed)
}

#[cfg(test)]
async fn run_blocking_with_timeout<T>(
    timeout: Duration,
    work: impl FnOnce() -> Result<T, ImageProcessorError> + Send + 'static,
) -> Result<T, ImageProcessorError>
where
    T: Send + 'static,
{
    run_blocking_with_slots(timeout, Arc::clone(&BLOCKING_IMAGE_SLOTS), work).await
}

#[cfg(test)]
async fn run_blocking_with_slots<T>(
    timeout: Duration,
    slots: Arc<tokio::sync::Semaphore>,
    work: impl FnOnce() -> Result<T, ImageProcessorError> + Send + 'static,
) -> Result<T, ImageProcessorError>
where
    T: Send + 'static,
{
    let permit = tokio::time::timeout(timeout, slots.acquire_owned())
        .await
        .map_err(|_| ImageProcessorError::ProcessingFailed)?
        .map_err(|_| ImageProcessorError::ProcessingFailed)?;
    run_blocking_with_permit_and_timeout(timeout, permit, work).await
}

async fn run_blocking_with_permit<T>(
    permit: tokio::sync::OwnedSemaphorePermit,
    work: impl FnOnce() -> Result<T, ImageProcessorError> + Send + 'static,
) -> Result<T, ImageProcessorError>
where
    T: Send + 'static,
{
    run_blocking_with_permit_and_timeout(PROCESSING_TIMEOUT, permit, work).await
}

async fn run_blocking_with_permit_and_timeout<T>(
    timeout: Duration,
    permit: tokio::sync::OwnedSemaphorePermit,
    work: impl FnOnce() -> Result<T, ImageProcessorError> + Send + 'static,
) -> Result<T, ImageProcessorError>
where
    T: Send + 'static,
{
    let processing = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    });
    tokio::time::timeout(timeout, processing)
        .await
        .map_err(|_| ImageProcessorError::ProcessingFailed)?
        .map_err(|_| ImageProcessorError::ProcessingFailed)?
}

