//! Fetch utilities for parallel resource retrieval with timeout and retry.

use anyhow::{Result, anyhow};
use std::{sync::LazyLock, time::Duration};
use tokio::sync::Semaphore;

const MAX_CONCURRENT_CORE_FETCHES: usize = 8;
const MAX_CONCURRENT_SECONDARY_FETCHES: usize = 4;

pub(crate) static CORE_FETCH_SEMAPHORE: LazyLock<Semaphore> =
    LazyLock::new(|| Semaphore::new(MAX_CONCURRENT_CORE_FETCHES));
pub(crate) static SECONDARY_FETCH_SEMAPHORE: LazyLock<Semaphore> =
    LazyLock::new(|| Semaphore::new(MAX_CONCURRENT_SECONDARY_FETCHES));

/// Per-resource fetch timeout in seconds.
const FETCH_TIMEOUT_SECS: u64 = 10;
/// Retry transient transport failures before surfacing errors.
const TRANSIENT_RETRY_ATTEMPTS: usize = 3;
const TRANSIENT_RETRY_DELAY_MS: u64 = 150;

/// Wraps a future with a semaphore permit, timeout, and transient-error retry.
pub(crate) async fn fetch_with_timeout<T, F, Fut>(
    label: &'static str,
    semaphore: &Semaphore,
    make_fut: F,
) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    for attempt in 0..=TRANSIENT_RETRY_ATTEMPTS {
        let _permit = semaphore
            .acquire()
            .await
            .map_err(|_| anyhow!("resource fetch coordinator shut down"))?;
        match tokio::time::timeout(Duration::from_secs(FETCH_TIMEOUT_SECS), make_fut()).await {
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(err)) => {
                if attempt < TRANSIENT_RETRY_ATTEMPTS && is_transient_send_request_error(&err) {
                    drop(_permit);
                    tokio::time::sleep(Duration::from_millis(TRANSIENT_RETRY_DELAY_MS)).await;
                    continue;
                }
                return Err(err);
            }
            Err(_) => {
                if attempt < TRANSIENT_RETRY_ATTEMPTS {
                    drop(_permit);
                    tokio::time::sleep(Duration::from_millis(TRANSIENT_RETRY_DELAY_MS)).await;
                    continue;
                }
                return Err(anyhow!(
                    "timed out fetching {label} ({}s)",
                    FETCH_TIMEOUT_SECS
                ));
            }
        }
    }
    unreachable!()
}

/// Returns `true` if the error looks like a transient transport failure
/// that is worth retrying (e.g. connection reset, broken pipe).
pub(crate) fn is_transient_send_request_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        if let Some(io_err) = cause.downcast_ref::<std::io::Error>() {
            return matches!(
                io_err.kind(),
                std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::TimedOut
            );
        }
        let text = cause.to_string();
        text.contains("SendRequest")
            || text.contains("Connection refused")
            || text.contains("connection reset")
            || text.contains("connection closed")
            || text.contains("broken pipe")
            || text.contains("timed out sending request")
    })
}

/// Conditionally fetches a resource if `enabled` is true.
pub(crate) async fn maybe_fetch<T, F, Fut>(
    enabled: bool,
    label: &'static str,
    semaphore: &Semaphore,
    make_fut: F,
) -> Option<Result<T>>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    if enabled {
        Some(fetch_with_timeout(label, semaphore, make_fut).await)
    } else {
        None
    }
}

/// Applies a vec-valued fetch result to a snapshot slot, recording errors.
pub(crate) fn apply_vec_fetch_result<T>(
    slot: &mut Vec<T>,
    result: Option<Result<Vec<T>>>,
    label: &str,
    errors: &mut Vec<String>,
    total_fetches: &mut usize,
) {
    let Some(result) = result else {
        return;
    };
    *total_fetches += 1;
    match result {
        Ok(items) => *slot = items,
        Err(err) => {
            errors.push(format!("{label}: {err}"));
        }
    }
}

/// Applies an optional fetch result, recording errors and returning the value on success.
pub(crate) fn apply_optional_fetch_result<T>(
    result: Option<Result<T>>,
    label: &str,
    errors: &mut Vec<String>,
    total_fetches: &mut usize,
) -> Option<T> {
    let result = result?;
    *total_fetches += 1;
    match result {
        Ok(value) => Some(value),
        Err(err) => {
            errors.push(format!("{label}: {err}"));
            None
        }
    }
}
