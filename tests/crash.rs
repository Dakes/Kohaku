//! Crashed lock holders, as child processes (data-storage: Instance lock). Its own test
//! binary, run one test at a time: a forked child briefly holds copies of the parent's
//! descriptors, lock files included, until it executes.

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::net::SocketAddr;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::{Arc, Mutex};

use kohaku::db::DataDir;
use kohaku::db::lock::{InstanceLock, LockError};
use kohaku::mail::{Mailer, Outgoing, SendFuture};
use kohaku::serve::{Listen, MailerChoice, serve};
use support::*;
use tokio::sync::oneshot;

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

struct NullMailer;

impl Mailer for NullMailer {
    fn send(&self, _: Outgoing) -> SendFuture<'_> {
        Box::pin(async { Ok(()) })
    }
}

fn null() -> MailerChoice {
    MailerChoice::Given(Arc::new(NullMailer))
}

/// Snapshot of a directory: name → (inode, size, modification time).
fn listing(dir: &Path) -> BTreeMap<String, (u64, u64, i64)> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            let m = fs::symlink_metadata(e.path()).unwrap();
            let name = e.file_name().into_string().unwrap();
            (
                name,
                (
                    m.ino(),
                    m.size(),
                    m.mtime_nsec() + m.mtime() * 1_000_000_000,
                ),
            )
        })
        .collect()
}

#[test]
#[ignore = "helper process for a_second_lock_is_refused_and_a_crashed_holder_releases_it"]
fn child_holds_lock() {
    let Some(root) = child_argument() else { return };
    let lock = InstanceLock::acquire(&DataDir::new(root)).unwrap();
    std::mem::forget(lock);
    child_ready_and_wait();
}

#[test]
fn a_second_lock_is_refused_and_a_crashed_holder_releases_it() {
    let _serial = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (dir, data) = data_dir();
    let mut child = spawn_child("child_holds_lock", dir.path().to_str().unwrap());
    assert_eq!(mode(&data.lock()), 0o600);
    let inode = fs::metadata(data.lock()).unwrap().ino();
    let before = listing(dir.path());
    let error = InstanceLock::acquire(&data).err().unwrap();
    assert!(matches!(error, LockError::Held));
    assert!(
        error
            .to_string()
            .contains("another Kohaku process is using /data")
    );
    assert_eq!(listing(dir.path()), before);
    child.kill().unwrap();
    child.wait().unwrap();
    let lock = InstanceLock::acquire(&data).unwrap();
    assert_eq!(fs::metadata(data.lock()).unwrap().ino(), inode);
    drop(lock);
}

#[test]
#[ignore = "helper process for a_crashed_server_releases_the_lock"]
fn child_serves() {
    let Some(root) = child_argument() else { return };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        serve(
            serve_config(&[]),
            DataDir::new(root),
            Listen::Address("127.0.0.1:0".parse().unwrap()),
            null(),
            |_| child_ready_and_wait(),
            std::future::pending(),
        )
        .await
        .unwrap();
    });
}

#[test]
fn a_crashed_server_releases_the_lock() {
    let _serial = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (dir, data) = data_dir();
    let mut child = spawn_child("child_serves", dir.path().to_str().unwrap());
    assert!(matches!(InstanceLock::acquire(&data), Err(LockError::Held)));
    let inode = fs::metadata(data.lock()).unwrap().ino();
    child.kill().unwrap();
    child.wait().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let (stop, stopped) = oneshot::channel::<()>();
        let (listening, bound) = oneshot::channel::<SocketAddr>();
        let task = tokio::spawn(serve(
            serve_config(&[]),
            data.clone(),
            Listen::Address("127.0.0.1:0".parse().unwrap()),
            null(),
            move |addr| {
                let _ = listening.send(addr);
            },
            async move {
                let _ = stopped.await;
            },
        ));
        let addr = bound.await.unwrap();
        assert_eq!(
            wire_request(addr, "GET", "x", "/healthz", &[]).await,
            Some(200)
        );
        let _ = stop.send(());
        task.await.unwrap().unwrap();
    });
    assert_eq!(fs::metadata(data.lock()).unwrap().ino(), inode);
}
