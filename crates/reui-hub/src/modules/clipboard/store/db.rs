//! The `clipboard-db` thread that owns the [`Store`]; [`Db`] queues calls to it in order.

use std::sync::mpsc as std_mpsc;

use rusqlite::Result;

use super::Store;

type Job = Box<dyn FnOnce(&mut Store) + Send>;

/// Handle to the DB thread. Calls queue in order; the thread ends when every handle is dropped.
#[derive(Clone)]
pub struct Db {
    jobs: std_mpsc::Sender<Job>,
}

impl Db {
    /// Opens the store on its own thread; `None` if it can't be opened.
    pub async fn start(open: impl FnOnce() -> Result<Store> + Send + 'static) -> Option<Db> {
        let (jobs, queue) = std_mpsc::channel::<Job>();
        let (ready_tx, ready) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("clipboard-db".into())
            .spawn(move || {
                let mut store = match open() {
                    Ok(store) => store,
                    Err(e) => {
                        eprintln!("clipboard: can't open history: {e}");
                        _ = ready_tx.send(false);
                        return;
                    }
                };
                _ = ready_tx.send(true);
                // A panic in one operation must not end history: this thread is the only
                // way to the database, and losing it would silently stop every save.
                for job in queue {
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&mut store))).is_err() {
                        eprintln!("clipboard: a history operation failed unexpectedly; skipped it");
                    }
                }
            })
            .ok()?;
        ready.await.unwrap_or(false).then_some(Db { jobs })
    }

    /// Runs `f` on the DB thread. Errors are logged there and come back as `None`.
    pub async fn call<T: Send + 'static>(&self, f: impl FnOnce(&mut Store) -> Result<T> + Send + 'static) -> Option<T> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let job: Job = Box::new(move |store| _ = tx.send(f(store).map_err(|e| eprintln!("clipboard: {e}")).ok()));
        self.jobs.send(job).ok()?;
        rx.await.ok().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::text;
    use super::super::Query;
    use super::*;

    #[tokio::test]
    async fn the_db_thread_runs_calls() {
        let db = Db::start(Store::in_memory).await.unwrap();
        let id = db.call(|s| s.add(text("x"), 1)).await.flatten();
        assert!(id.is_some());
        assert_eq!(db.call(|s| s.page(&Query::default(), 60)).await.unwrap().items.len(), 1);
        assert!(Db::start(|| Err(rusqlite::Error::InvalidQuery)).await.is_none());
    }
}
