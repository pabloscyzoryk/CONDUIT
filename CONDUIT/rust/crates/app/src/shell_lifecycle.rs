//! Native shell ownership is separate from the running services.
//! A window failure must not silently destroy the runtime on a portable VPS.

pub fn finish_native_session(
    run_window: impl FnOnce() -> anyhow::Result<()>,
    open_fallback: impl FnOnce(anyhow::Error) -> anyhow::Result<()>,
    wait_for_stop: impl FnOnce(),
    shutdown: impl FnOnce(),
) {
    if let Err(error) = run_window() {
        // The adapter logs browser failures. Either outcome keeps services alive
        // and the known local URL available until an explicit stop signal.
        let _ = open_fallback(error);
        wait_for_stop();
    }
    shutdown();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[test]
    fn normal_window_close_shuts_down_once_without_wait_or_browser() {
        let calls=RefCell::new(Vec::new());
        finish_native_session(
            || { calls.borrow_mut().push("window"); Ok(()) },
            |_| { panic!("normal close must not open a browser") },
            || panic!("normal close must not enter headless wait"),
            || calls.borrow_mut().push("shutdown"),
        );
        assert_eq!(*calls.borrow(),["window","shutdown"]);
    }

    #[test]
    fn failed_window_preserves_services_until_stop_even_when_browser_also_fails() {
        for browser_ok in [true,false] {
            let calls=RefCell::new(Vec::new());
            let alive=Cell::new(true);
            finish_native_session(
                || { calls.borrow_mut().push("window"); Err(anyhow::anyhow!("synthetic missing WebView2")) },
                |reason| {
                    assert!(reason.to_string().contains("WebView2")); assert!(alive.get());
                    calls.borrow_mut().push("browser");
                    if browser_ok { Ok(()) } else { Err(anyhow::anyhow!("synthetic browser unavailable")) }
                },
                || { assert!(alive.get()); calls.borrow_mut().push("wait"); },
                || { assert!(alive.replace(false)); calls.borrow_mut().push("shutdown"); },
            );
            assert!(!alive.get());
            assert_eq!(*calls.borrow(),["window","browser","wait","shutdown"]);
        }
    }
}
