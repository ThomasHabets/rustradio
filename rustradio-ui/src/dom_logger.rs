//! Buffered logging: shared-memory producers never touch the DOM.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use log::{Level, LevelFilter, Log, Metadata, Record};
use wasm_bindgen::prelude::*;

use crate::{ApplicationSpecific, WorkerToMain};

// We should never reach max messages. 1000 log lines in 250ms? Unlikely.
const MAX_LOG_MESSAGES: usize = 1000;
const LOG_PERIOD_MS: i32 = 250;

#[derive(Default)]
struct Pending {
    lines: VecDeque<(Level, String)>,
    dropped: usize,
}

struct DomConsoleLogger {
    level: LevelFilter,
    pending: Arc<Mutex<Pending>>,
}

impl Log for DomConsoleLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= self.level
    }
    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let mut pending = self.pending.lock().unwrap();
        if pending.lines.len() == MAX_LOG_MESSAGES {
            pending.dropped += 1;
            return;
        }
        pending
            .lines
            .push_back((record.level(), record.args().to_string()));
    }
    fn flush(&self) {}
}

pub fn init_logging<App>(
    element_id: impl Into<String>,
    level: LevelFilter,
) -> Result<(), log::SetLoggerError>
where
    App: ApplicationSpecific + 'static,
{
    let pending = Arc::new(Mutex::new(Pending::default()));
    log::set_boxed_logger(Box::new(DomConsoleLogger {
        level,
        pending: pending.clone(),
    }))?;
    log::set_max_level(level);
    let element_id = element_id.into();
    let mut history = VecDeque::new();
    let callback = Closure::<dyn FnMut()>::new(move || {
        let Pending { mut lines, dropped } = {
            let mut pending = pending.lock().unwrap();
            if pending.lines.is_empty() && pending.dropped == 0 {
                return;
            }
            std::mem::take(&mut *pending)
        };
        if dropped > 0 {
            lines.push_back((Level::Warn, format!("Suppressed {dropped} log messages")));
        }
        let document = web_sys::window().and_then(|w| w.document());

        // Console log only once per level, for performance.
        for level in [
            Level::Error,
            Level::Warn,
            Level::Info,
            Level::Debug,
            Level::Trace,
        ] {
            let text = lines
                .iter()
                .filter(|(l, _)| *l == level)
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                continue;
            }
            let text = JsValue::from_str(&text);
            match level {
                Level::Error => web_sys::console::error_1(&text),
                Level::Warn => web_sys::console::warn_1(&text),
                Level::Info => web_sys::console::info_1(&text),
                _ => web_sys::console::debug_1(&text),
            }
        }
        for (level, line) in lines {
            if document.is_none() {
                // Fallback for workers with a separately installed logger.
                let _ = crate::worker::post_message::<WorkerToMain<App>>(&WorkerToMain::LogLine {
                    level,
                    line,
                });
            } else {
                history.push_back(format!("[{level}] {line}\n"));
            }
        }
        while history.len() > MAX_LOG_MESSAGES {
            history.pop_front();
        }
        if let Some(el) = document.and_then(|d| d.get_element_by_id(&element_id)) {
            let text: String = history.iter().map(String::as_str).collect();
            el.set_text_content(Some(&text));
            #[allow(clippy::useless_conversion)]
            el.set_scroll_top(el.scroll_height().into());
        }
    });
    // Schedule periodic logging of batched messages.
    let scheduled = if let Some(window) = web_sys::window() {
        window.set_interval_with_callback_and_timeout_and_arguments_0(
            callback.as_ref().unchecked_ref(),
            LOG_PERIOD_MS,
        )
    } else {
        js_sys::global()
            .unchecked_into::<web_sys::WorkerGlobalScope>()
            .set_interval_with_callback_and_timeout_and_arguments_0(
                callback.as_ref().unchecked_ref(),
                LOG_PERIOD_MS,
            )
    };
    if scheduled.is_ok() {
        callback.forget();
    }
    Ok(())
}
