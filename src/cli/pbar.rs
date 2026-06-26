use indicatif::{MultiProgress, ProgressBar, ProgressStyle, WeakProgressBar};
struct TickerHandle {
    _handle: compio::runtime::JoinHandle<()>,
}

fn spawn_pb_ticker(pb: WeakProgressBar, interval: std::time::Duration) -> TickerHandle {
    let h = compio::runtime::spawn(async move {
        while let Some(pb) = pb.upgrade() {
            pb.tick();
            compio::time::sleep(interval).await;
        }
    });

    TickerHandle { _handle: h }
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
    _ticker: TickerHandle,
}

impl std::ops::Deref for AsyncSpinner {
    type Target = ProgressBar;
    fn deref(&self) -> &Self::Target {
        &self.pb
    }
}

fn new_async_spinner(pb: ProgressBar) -> AsyncSpinner {
    let w = pb.downgrade();
    let ticker = spawn_pb_ticker(w, std::time::Duration::from_millis(100));
    AsyncSpinner {
        pb,
        _ticker: ticker,
    }
}

/// Create a new spinner with a default style (standalone, not attached to a [`MultiProgress`]).
pub fn new_spinner() -> AsyncSpinner {
    new_async_spinner(ProgressBar::new_spinner())
}

/// Create a spinner registered on the given [`MultiProgress`] group.
pub fn new_spinner_on(multi: &MultiProgress) -> AsyncSpinner {
    new_async_spinner(multi.add(ProgressBar::new_spinner()))
}

/// Create a new progress bar with a given length and a default style
pub fn new(pb_len: u64) -> ProgressBar {
    let pb = ProgressBar::new(pb_len);
    pb.set_style(pb_style());
    pb
}
