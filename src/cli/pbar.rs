use indicatif::{ProgressBar, ProgressStyle, WeakProgressBar};
struct TickerHandle {
    #[allow(dead_code)]
    handle: compio::runtime::JoinHandle<()>,
}

fn spawn_pb_ticker(pb: WeakProgressBar, interval: std::time::Duration) -> TickerHandle {
    let h = compio::runtime::spawn(async move {
        while let Some(pb) = pb.upgrade() {
            pb.tick();
            compio::time::sleep(interval).await;
        }
    });

    TickerHandle { handle: h }
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
    #[allow(dead_code)]
    ticker: TickerHandle,
}

impl std::ops::Deref for AsyncSpinner {
    type Target = ProgressBar;
    fn deref(&self) -> &Self::Target {
        &self.pb
    }
}

/// Create a new spinner with a default style
pub fn new_spinner() -> AsyncSpinner {
    let pb = ProgressBar::new_spinner();
    let w = pb.downgrade();
    let ticker = spawn_pb_ticker(w, std::time::Duration::from_millis(100));
    AsyncSpinner { pb, ticker }
}

/// Create a new progress bar with a given length and a default style
pub fn new(pb_len: u64) -> ProgressBar {
    let pb = ProgressBar::new(pb_len);
    pb.set_style(pb_style());
    pb
}
