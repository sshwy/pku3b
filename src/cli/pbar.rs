use futures_util::FutureExt;
use indicatif::{ProgressBar, ProgressStyle};
struct TickerHandle {
    tx: futures_channel::oneshot::Sender<()>,
    handle: compio::runtime::JoinHandle<()>,
}

impl TickerHandle {
    async fn stop(self) {
        let _ = self.tx.send(());
        self.handle.await.unwrap()
    }
}

fn spawn_pb_ticker(pb: ProgressBar, interval: std::time::Duration) -> TickerHandle {
    let (tx, mut rx) = futures_channel::oneshot::channel::<()>();
    let h = compio::runtime::spawn(async move {
        loop {
            pb.tick();
            let wait = compio::time::sleep(interval);
            let mut wait = std::pin::pin!(wait.fuse());
            futures_util::select! {
                _ = rx => break,
                _ = wait => (),
            }
        }
    });

    TickerHandle { tx, handle: h }
}

fn pb_style() -> ProgressStyle {
    // a trailing space is left for the cursor
    ProgressStyle::with_template(
        "{prefix} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ",
    )
    .unwrap()
    .progress_chars("=> ")
}

/// Progress bar that ticks asynchronously
pub struct AsyncSpinner {
    pb: ProgressBar,
    ticker: TickerHandle,
}

impl std::ops::Deref for AsyncSpinner {
    type Target = ProgressBar;
    fn deref(&self) -> &Self::Target {
        &self.pb
    }
}

impl AsyncSpinner {
    pub async fn finish_and_clear(self) {
        self.ticker.stop().await;
        self.pb.finish_and_clear();
    }
}

/// Create a new spinner with a default style
pub fn new_spinner() -> AsyncSpinner {
    let pb = ProgressBar::new_spinner();
    let ticker = spawn_pb_ticker(pb.clone(), std::time::Duration::from_millis(100));
    AsyncSpinner { pb, ticker }
}

/// Create a new progress bar with a given length and a default style
pub fn new(pb_len: u64) -> ProgressBar {
    let pb = ProgressBar::new(pb_len);
    pb.set_style(pb_style());
    pb
}
