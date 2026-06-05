//! Post-publish index-provider fanout.
//!
//! Graph commits publish the immutable snapshot before provider callbacks run,
//! so provider failures can never roll a commit back. This module keeps that
//! boundary explicit and isolates provider panics/errors from the committer
//! thread after publication.

use std::sync::Arc;

use selene_core::Change;

use crate::index_provider::IndexProvider;

const SENTINEL_PROVIDER_TAG: &str = "<unknown>";

/// Fan out committed changes to every registered provider.
#[tracing::instrument(
    name = "selene.graph.notify_providers",
    skip(providers, changes),
    fields(provider_count = providers.len(), change_count = changes.len(), generation)
)]
pub(crate) fn notify_providers(
    providers: &[Arc<dyn IndexProvider>],
    generation: u64,
    changes: &[Change],
) {
    for provider in providers {
        let handles_change_batches =
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                provider.handles_change_batches()
            })) {
                Ok(value) => value,
                Err(payload) => {
                    let payload = crate::panic_payload::describe(&payload);
                    tracing::error!(
                        provider_tag = %SENTINEL_PROVIDER_TAG,
                        payload = %payload,
                        "index provider handles_change_batches() panicked after graph commit; \
                         skipping provider fanout for this batch",
                    );
                    continue;
                }
            };
        if handles_change_batches {
            if notify_provider_batch(provider, generation, changes) {
                notify_provider_commit_applied(provider, generation);
            }
            continue;
        }
        let mut applied_all_changes = true;
        for change in changes {
            applied_all_changes &= notify_provider_change(provider, change);
        }
        if applied_all_changes {
            notify_provider_commit_applied(provider, generation);
        }
    }
}

fn notify_provider_change(provider: &Arc<dyn IndexProvider>, change: &Change) -> bool {
    let tag =
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| provider.provider_tag())) {
            Ok(tag) => tag,
            Err(payload) => {
                let payload = crate::panic_payload::describe(&payload);
                tracing::error!(
                    provider_tag = %SENTINEL_PROVIDER_TAG,
                    ?change,
                    payload = %payload,
                    "index provider provider_tag() panicked after graph commit; \
                     skipping on_change for this change",
                );
                return false;
            }
        };

    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| provider.on_change(change)));
    match outcome {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            tracing::error!(
                provider_tag = %tag,
                error = %error,
                ?change,
                "index provider on_change failed after graph commit; continuing",
            );
            false
        }
        Err(panic_payload) => {
            let payload = crate::panic_payload::describe(&panic_payload);
            tracing::error!(
                provider_tag = %tag,
                ?change,
                payload = %payload,
                "index provider on_change panicked after graph commit; continuing",
            );
            false
        }
    }
}

fn notify_provider_batch(
    provider: &Arc<dyn IndexProvider>,
    generation: u64,
    changes: &[Change],
) -> bool {
    let tag =
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| provider.provider_tag())) {
            Ok(tag) => tag,
            Err(payload) => {
                let payload = crate::panic_payload::describe(&payload);
                tracing::error!(
                    provider_tag = %SENTINEL_PROVIDER_TAG,
                    change_count = changes.len(),
                    payload = %payload,
                    "index provider provider_tag() panicked after graph commit; \
                     skipping on_changes for this batch",
                );
                return false;
            }
        };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        provider.on_changes(changes)
    }));
    match outcome {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            tracing::error!(
                provider_tag = %tag,
                generation,
                error = %error,
                change_count = changes.len(),
                "index provider on_changes failed after graph commit; continuing",
            );
            false
        }
        Err(panic_payload) => {
            let payload = crate::panic_payload::describe(&panic_payload);
            tracing::error!(
                provider_tag = %tag,
                generation,
                change_count = changes.len(),
                payload = %payload,
                "index provider on_changes panicked after graph commit; continuing",
            );
            false
        }
    }
}

fn notify_provider_commit_applied(provider: &Arc<dyn IndexProvider>, generation: u64) {
    let tag =
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| provider.provider_tag())) {
            Ok(tag) => tag,
            Err(payload) => {
                let payload = crate::panic_payload::describe(&payload);
                tracing::error!(
                    provider_tag = %SENTINEL_PROVIDER_TAG,
                    generation,
                    payload = %payload,
                    "index provider provider_tag() panicked after graph commit; \
                     skipping generation watermark",
                );
                return;
            }
        };

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        provider.on_commit_applied(generation)
    }));
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            tracing::error!(
                provider_tag = %tag,
                generation,
                error = %error,
                "index provider on_commit_applied failed after graph commit; continuing",
            );
        }
        Err(panic_payload) => {
            let payload = crate::panic_payload::describe(&panic_payload);
            tracing::error!(
                provider_tag = %tag,
                generation,
                payload = %payload,
                "index provider on_commit_applied panicked after graph commit; continuing",
            );
        }
    }
}
