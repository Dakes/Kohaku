//! The SQLite database: connection settings, the writer and reader pool, and the
//! check-and-consume helpers (data-storage; change foundation D7).

pub mod backup;
pub mod current;
pub mod lock;
pub mod migrate;
pub mod paths;

use std::fs::OpenOptions;
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use rusqlite::{Connection, OpenFlags, Params, Transaction, TransactionBehavior};
use tokio::sync::Semaphore;

pub use paths::DataDir;

/// Read connections per `serve` process.
pub const READERS: usize = 4;

/// How long a statement waits for a lock held by another connection.
const BUSY_TIMEOUT_MS: i64 = 5000;

/// WAL size kept after a checkpoint.
const JOURNAL_SIZE_LIMIT: i64 = 64 * 1024 * 1024;

/// Which settings a connection gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The one connection carrying every write; `secure_delete=ON`.
    Writer,
    /// `query_only=ON`, so a write through it fails.
    Reader,
    /// A command's own connection (backup): the common settings only.
    Command,
}

/// Opens the existing database file with the settings of `role`; never creates it.
pub fn open(path: &Path, role: Role) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.pragma_update(None, "busy_timeout", BUSY_TIMEOUT_MS)?;
    let mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(rusqlite::Error::InvalidQuery);
    }
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.query_row(
        &format!("PRAGMA journal_size_limit={JOURNAL_SIZE_LIMIT}"),
        [],
        |_| Ok(()),
    )?;
    match role {
        Role::Writer => conn.pragma_update(None, "secure_delete", true)?,
        Role::Reader => conn.pragma_update(None, "query_only", true)?,
        Role::Command => {}
    }
    Ok(conn)
}

/// Creates the database file with mode 0600 if missing, so SQLite (and its `-wal` and
/// `-shm` files, which copy the mode) never makes it world-readable.
pub fn create_file(path: &Path) -> io::Result<()> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map(drop)
}

/// The connections of a `serve` process: one writer and [`READERS`] readers, each
/// kind behind a semaphore so bursts wait asynchronously instead of parking threads,
/// and the watcher, a reader of its own that builds the host map.
pub struct Db {
    writer: Arc<Mutex<Connection>>,
    write_permits: Arc<Semaphore>,
    readers: Arc<Mutex<Vec<Connection>>>,
    read_permits: Arc<Semaphore>,
    watcher: Arc<Mutex<Connection>>,
}

impl Db {
    /// Takes the (already migrated) writer and opens the readers and the watcher.
    pub fn new(writer: Connection, path: &Path) -> rusqlite::Result<Db> {
        let readers = (0..READERS)
            .map(|_| open(path, Role::Reader))
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Db {
            writer: Arc::new(Mutex::new(writer)),
            write_permits: Arc::new(Semaphore::new(1)),
            readers: Arc::new(Mutex::new(readers)),
            read_permits: Arc::new(Semaphore::new(READERS)),
            watcher: Arc::new(Mutex::new(open(path, Role::Reader)?)),
        })
    }

    /// Runs `work` in one `BEGIN IMMEDIATE` transaction on the writer, committed when it
    /// returns `Ok`. Dropping the future while it waits for the writer cancels the
    /// wait; once `work` has started it always runs to completion.
    pub async fn write<T, E, F>(&self, work: F) -> Result<T, E>
    where
        F: FnOnce(&Transaction<'_>) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<rusqlite::Error> + Send + 'static,
    {
        let permit = Arc::clone(&self.write_permits)
            .acquire_owned()
            .await
            .expect("the write semaphore is never closed");
        let writer = Arc::clone(&self.writer);
        run_blocking(move || {
            let _permit = permit;
            let mut conn = writer.lock().unwrap_or_else(PoisonError::into_inner);
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let value = work(&tx)?;
            tx.commit()?;
            Ok(value)
        })
        .await
    }

    /// Runs `work` on a free reader.
    pub async fn read<T, E, F>(&self, work: F) -> Result<T, E>
    where
        F: FnOnce(&Connection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
    {
        let permit = Arc::clone(&self.read_permits)
            .acquire_owned()
            .await
            .expect("the read semaphore is never closed");
        let readers = Arc::clone(&self.readers);
        run_blocking(move || {
            let _permit = permit;
            let conn = Pooled::take(&readers);
            work(conn.get())
        })
        .await
    }

    /// Runs `work` on the watcher connection; one caller at a time, so work that reads
    /// and then publishes what it read cannot interleave with another such run.
    pub async fn with_watcher<T, F>(&self, work: F) -> T
    where
        F: FnOnce(&Connection) -> T + Send + 'static,
        T: Send + 'static,
    {
        let watcher = Arc::clone(&self.watcher);
        run_blocking(move || {
            let conn = watcher.lock().unwrap_or_else(PoisonError::into_inner);
            work(&conn)
        })
        .await
    }

    /// Runs `work` on the writer outside any transaction (checkpoints, which cannot run
    /// inside one).
    pub async fn with_writer<T, F>(&self, work: F) -> T
    where
        F: FnOnce(&Connection) -> T + Send + 'static,
        T: Send + 'static,
    {
        let permit = Arc::clone(&self.write_permits)
            .acquire_owned()
            .await
            .expect("the write semaphore is never closed");
        let writer = Arc::clone(&self.writer);
        run_blocking(move || {
            let _permit = permit;
            let conn = writer.lock().unwrap_or_else(PoisonError::into_inner);
            work(&conn)
        })
        .await
    }
}

/// A reader taken from the pool, put back when dropped (panics included).
struct Pooled<'a> {
    pool: &'a Mutex<Vec<Connection>>,
    conn: Option<Connection>,
}

impl<'a> Pooled<'a> {
    fn take(pool: &'a Mutex<Vec<Connection>>) -> Pooled<'a> {
        // The semaphore admits at most READERS holders, so one is always free.
        let conn = pool
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop()
            .expect("a read permit guarantees a free reader");
        Pooled {
            pool,
            conn: Some(conn),
        }
    }

    fn get(&self) -> &Connection {
        self.conn.as_ref().expect("present until dropped")
    }
}

impl Drop for Pooled<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            self.pool
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(conn);
        }
    }
}

/// `spawn_blocking`, re-raising a panic of `work` in the caller.
async fn run_blocking<T, F>(work: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(value) => value,
        Err(error) => std::panic::resume_unwind(error.into_panic()),
    }
}

/// Runs one conditional statement that changes at most one row; `true` only when it
/// changed one. The statement's own condition is the check (data-storage: Atomic limit
/// enforcement), so a concurrent attempt can never also succeed.
pub fn consume_one<P: Params>(
    tx: &Transaction<'_>,
    sql: &str,
    params: P,
) -> rusqlite::Result<bool> {
    match tx.execute(sql, params)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(rusqlite::Error::StatementChangedRows(2)),
    }
}

/// Runs one conditional statement with `RETURNING`; `None` when its condition held for
/// no row.
pub fn consume_returning<T, P, F>(
    tx: &Transaction<'_>,
    sql: &str,
    params: P,
    map: F,
) -> rusqlite::Result<Option<T>>
where
    P: Params,
    F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut statement = tx.prepare(sql)?;
    let mut rows = statement.query(params)?;
    let value = match rows.next()? {
        Some(row) => Some(map(row)?),
        None => None,
    };
    if rows.next()?.is_some() {
        return Err(rusqlite::Error::StatementChangedRows(2));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// A new database with one STRICT table and a foreign key, as the writer.
    fn fresh(dir: &TempDir) -> (Connection, std::path::PathBuf) {
        let path = dir.path().join("kohaku.db");
        create_file(&path).unwrap();
        let conn = open(&path, Role::Writer).unwrap();
        conn.execute_batch(
            "CREATE TABLE parent (id INTEGER PRIMARY KEY) STRICT;
             CREATE TABLE child (id INTEGER PRIMARY KEY,
                 parent_id INTEGER NOT NULL REFERENCES parent (id)) STRICT;
             CREATE TABLE codes (id INTEGER PRIMARY KEY, used_at INTEGER) STRICT;
             CREATE TABLE limits (id INTEGER PRIMARY KEY, used INTEGER NOT NULL,
                 max INTEGER NOT NULL, CHECK (used <= max)) STRICT;",
        )
        .unwrap();
        (conn, path)
    }

    fn pragma<T: rusqlite::types::FromSql>(conn: &Connection, name: &str) -> T {
        conn.query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn connection_settings() {
        let dir = TempDir::new();
        let (writer, path) = fresh(&dir);
        let reader = open(&path, Role::Reader).unwrap();
        for (conn, secure, query_only) in [(&writer, 1, 0), (&reader, 0, 1)] {
            assert_eq!(pragma::<String>(conn, "journal_mode"), "wal");
            assert_eq!(pragma::<i64>(conn, "synchronous"), 1);
            assert_eq!(pragma::<i64>(conn, "foreign_keys"), 1);
            assert_eq!(pragma::<i64>(conn, "busy_timeout"), 5000);
            assert_eq!(pragma::<i64>(conn, "journal_size_limit"), 67_108_864);
            assert_eq!(pragma::<i64>(conn, "secure_delete"), secure);
            assert_eq!(pragma::<i64>(conn, "query_only"), query_only);
        }
        writer.execute("INSERT INTO parent VALUES (1)", []).unwrap();
        for file in ["kohaku.db", "kohaku.db-wal", "kohaku.db-shm"] {
            let meta = std::fs::metadata(dir.path().join(file)).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600, "{file}");
        }
        assert!(reader.execute("INSERT INTO parent VALUES (2)", []).is_err());
        assert!(
            writer
                .execute("INSERT INTO codes VALUES ('x', NULL)", [])
                .is_err(),
            "STRICT refuses a text id"
        );
        assert!(
            writer
                .execute("INSERT INTO child VALUES (1, 99)", [])
                .is_err(),
            "foreign keys are enforced"
        );
    }

    #[test]
    fn open_never_creates() {
        let dir = TempDir::new();
        assert!(open(&dir.path().join("kohaku.db"), Role::Command).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_reads_stay_within_the_connection_bound() {
        let dir = TempDir::new();
        let (writer, path) = fresh(&dir);
        let db = Arc::new(Db::new(writer, &path).unwrap());
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = (0..100)
            .map(|_| {
                let (db, active, peak) = (Arc::clone(&db), Arc::clone(&active), Arc::clone(&peak));
                tokio::spawn(async move {
                    db.read(move |conn| {
                        let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(now, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(5));
                        let n: i64 =
                            conn.query_row("SELECT count(*) FROM parent", [], |r| r.get(0))?;
                        active.fetch_sub(1, Ordering::SeqCst);
                        Ok::<_, rusqlite::Error>(n)
                    })
                    .await
                })
            })
            .collect();
        for task in tasks {
            assert_eq!(task.await.unwrap().unwrap(), 0);
        }
        assert_eq!(peak.load(Ordering::SeqCst), READERS);
        assert_eq!(db.readers.lock().unwrap().len(), READERS);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_write_waits_out_a_two_second_write_transaction() {
        let dir = TempDir::new();
        let (writer, path) = fresh(&dir);
        let db = Db::new(writer, &path).unwrap();
        let other = open(&path, Role::Command).unwrap();
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            other
                .execute_batch("BEGIN IMMEDIATE; INSERT INTO parent VALUES (1);")
                .unwrap();
            locked_tx.send(()).unwrap();
            std::thread::sleep(Duration::from_secs(2));
            other.execute_batch("COMMIT").unwrap();
        });
        locked_rx.recv().unwrap();
        let start = Instant::now();
        db.write(|tx| tx.execute("INSERT INTO parent VALUES (2)", []).map(drop))
            .await
            .unwrap();
        assert!(start.elapsed() >= Duration::from_millis(1500));
        holder.join().unwrap();
        let n: i64 = db
            .read(|c| c.query_row("SELECT count(*) FROM parent", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(n, 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn failed_work_rolls_back() {
        let dir = TempDir::new();
        let (writer, path) = fresh(&dir);
        let db = Db::new(writer, &path).unwrap();
        let result: Result<(), rusqlite::Error> = db
            .write(|tx| {
                tx.execute("INSERT INTO parent VALUES (1)", [])?;
                tx.execute("INSERT INTO parent VALUES (1)", []).map(drop)
            })
            .await;
        assert!(result.is_err());
        let n: i64 = db
            .read(|c| c.query_row("SELECT count(*) FROM parent", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn parallel_attempts() {
        let dir = TempDir::new();
        let (writer, path) = fresh(&dir);
        writer
            .execute_batch(
                "INSERT INTO codes VALUES (1, NULL); INSERT INTO limits VALUES (1, 5, 10);",
            )
            .unwrap();
        let db = Arc::new(Db::new(writer, &path).unwrap());

        // Every attempt's early reader check sees uses left before any write runs.
        let early: Vec<bool> = futures_join(
            (0..20)
                .map(|_| {
                    let db = Arc::clone(&db);
                    async move {
                        db.read(|c| {
                            c.query_row("SELECT used < max FROM limits WHERE id = 1", [], |r| {
                                r.get::<_, bool>(0)
                            })
                        })
                        .await
                        .unwrap()
                    }
                })
                .collect(),
        )
        .await;
        assert!(early.iter().all(|&left| left));

        let redeem = |db: Arc<Db>| async move {
            db.write(|tx| {
                consume_one(
                    tx,
                    "UPDATE codes SET used_at = 1 WHERE id = 1 AND used_at IS NULL",
                    [],
                )
            })
            .await
            .unwrap()
        };
        let take = |db: Arc<Db>| async move {
            db.write(|tx| {
                consume_returning(
                    tx,
                    "UPDATE limits SET used = used + 1 WHERE id = 1 AND used < max RETURNING used",
                    [],
                    |row| row.get::<_, i64>(0),
                )
            })
            .await
            .unwrap()
        };
        let redeemed = futures_join((0..20).map(|_| redeem(Arc::clone(&db))).collect()).await;
        let taken = futures_join((0..20).map(|_| take(Arc::clone(&db))).collect()).await;
        assert_eq!(redeemed.iter().filter(|&&ok| ok).count(), 1);
        let mut counts: Vec<i64> = taken.into_iter().flatten().collect();
        counts.sort_unstable();
        assert_eq!(counts, [6, 7, 8, 9, 10]);
        let used: i64 = db
            .read(|c| c.query_row("SELECT used FROM limits", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(used, 10);
    }

    /// Runs every future concurrently on the runtime and collects the results in order.
    async fn futures_join<T: Send + 'static>(
        futures: Vec<impl std::future::Future<Output = T> + Send + 'static>,
    ) -> Vec<T> {
        let handles: Vec<_> = futures.into_iter().map(tokio::spawn).collect();
        let mut out = Vec::new();
        for handle in handles {
            out.push(handle.await.unwrap());
        }
        out
    }
}
