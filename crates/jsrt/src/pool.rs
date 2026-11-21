//! 能力调用的工作线程池。
//!
//! 为什么需要它：V8 回调运行在插件线程上，绝不能阻塞等 HTTP/磁盘；
//! 而宿主（Tauri）主线程也不能被插件随意拖住。所以异步能力统一丢进这个池，
//! 完成后通过插件的 inbound 通道回投 `HostEvent::Reply`。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;

type Job = Box<dyn FnOnce() + Send + 'static>;

struct Pool {
    sender: crossbeam_channel::Sender<Job>,
    running: Arc<AtomicBool>,
    workers: usize,
    seq: Arc<AtomicUsize>,
}

impl Pool {
    fn with_workers(workers: usize) -> Arc<Self> {
        let (tx, rx) = crossbeam_channel::unbounded::<Job>();
        let running = Arc::new(AtomicBool::new(true));
        for i in 0..workers {
            let rx = rx.clone();
            let running = running.clone();
            let _ = thread::Builder::new()
                .name(format!("efto-capability-{i}"))
                .spawn(move || {
                    while running.load(Ordering::SeqCst) {
                        match rx.recv() {
                            Ok(job) => {
                                // 单个能力调用失败不应该带走整个池
                                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                                if let Err(p) = res {
                                    let msg = p
                                        .downcast_ref::<String>()
                                        .cloned()
                                        .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                                        .unwrap_or_else(|| "未知 panic".into());
                                    tracing::error!("能力调用线程 panic: {msg}");
                                }
                            }
                            Err(_) => break,
                        }
                    }
                });
        }
        Arc::new(Self {
            sender: tx,
            running,
            workers,
            seq: Arc::new(AtomicUsize::new(0)),
        })
    }

    fn submit(&self, job: Job) -> bool {
        if !self.running.load(Ordering::SeqCst) {
            return false;
        }
        self.sender.send(job).is_ok()
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

static POOL: OnceLock<Arc<Pool>> = OnceLock::new();

fn pool() -> Arc<Pool> {
    POOL.get_or_init(|| {
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .clamp(2, 8);
        tracing::debug!("能力调用线程池 workers={workers}");
        Pool::with_workers(workers)
    })
    .clone()
}

/// 提交一个异步能力调用。返回 false 表示池已关闭（进程退出中）。
pub fn submit<F: FnOnce() + Send + 'static>(job: F) -> bool {
    pool().submit(Box::new(job))
}

/// 便于测试与诊断：worker 数量。
pub fn worker_count() -> usize {
    pool().workers
}

/// 观测用：累计提交数（单调递增，不落盘）。
pub fn submitted_total() -> usize {
    pool().seq.load(Ordering::SeqCst)
}

/// 内部计数：submit 时 +1。
pub(crate) fn bump() {
    pool().seq.fetch_add(1, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn jobs_run_off_caller_thread() {
        let counter = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..20 {
            let c = counter.clone();
            let (done_tx, done_rx) = crossbeam_channel::bounded(1);
            assert!(submit(move || {
                c.fetch_add(1, Ordering::SeqCst);
                let _ = done_tx.send(());
            }));
            handles.push(done_rx);
        }
        for h in handles {
            h.recv_timeout(std::time::Duration::from_secs(5))
                .expect("任务应在超时内完成");
        }
        assert_eq!(counter.load(Ordering::SeqCst), 20);
    }

    #[test]
    fn panicking_job_does_not_kill_pool() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        submit(move || panic!("故意炸"));
        let c = tx.clone();
        submit(move || {
            c.send("still alive").ok();
        });
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)).ok(), Some("still alive"));
    }

    #[test]
    fn worker_count_is_sane() {
        let n = worker_count();
        assert!((2..=8).contains(&n), "workers={n}");
    }
}
