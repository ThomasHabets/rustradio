//! Log provider that logs both to the browser console and to an element in the
//! web page DOM.
use std::collections::VecDeque;

use log::{Level, LevelFilter, Log, Metadata, Record};
use wasm_bindgen::JsCast;
use web_sys::{HtmlElement, window};

use crate::{ApplicationSpecific, WorkerToMain};

const MAX_LOG_MESSAGES: usize = 1000;

fn console_log(s: impl AsRef<str>) {
    web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(s.as_ref()));
}

struct DomConsoleLogger<App: ApplicationSpecific> {
    level: LevelFilter,
    log_lines: std::sync::Mutex<VecDeque<String>>,
    element_id: String,
    _app: std::marker::PhantomData<fn() -> App>,
}

impl<App: ApplicationSpecific> Log for DomConsoleLogger<App> {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let line = format!("[{}] {}", record.level(), record.args());

        // Also log to browser console.
        match record.level() {
            Level::Error => web_sys::console::error_1(&line.clone().into()),
            Level::Warn => web_sys::console::warn_1(&line.clone().into()),
            Level::Info => web_sys::console::info_1(&line.clone().into()),
            Level::Debug => web_sys::console::log_1(&line.clone().into()),
            Level::Trace => web_sys::console::debug_1(&line.clone().into()),
        }

        // DOM sink.
        //
        // TODO: can we cache this JS object? Or what happens if it's GC'd?

        let Some(document) = window().and_then(|w| w.document()) else {
            if let Err(e) =
                crate::worker::post_message::<WorkerToMain<App>>(&WorkerToMain::LogLine {
                    level: record.level(),
                    line: record.args().to_string(),
                })
            {
                console_log(format!("Error posting log message from worker: {e:?}"));
                console_log(format!("Worker console fallback: {line}"));
            }
            return;
        };

        let Some(el) = document.get_element_by_id(&self.element_id) else {
            return;
        };

        let Ok(el) = el.dyn_into::<HtmlElement>() else {
            return;
        };

        // Not that we expect to be multithreaded, but hold the lock a
        // shorter time anyway.
        let content = {
            let mut lines = self.log_lines.lock().unwrap();
            lines.push_back(line);
            while lines.len() > MAX_LOG_MESSAGES {
                lines.pop_front();
            }

            let mut content = String::new();
            for line in lines.iter() {
                content.push_str(line);
                content.push('\n');
            }
            content
        };
        el.set_inner_text(&content);

        // Looks like this type varies, so either into() is needed, or in clippy
        // warns.
        #[allow(clippy::useless_conversion)]
        {
            el.set_scroll_top(el.scroll_height().into());
        }
    }

    fn flush(&self) {}
}

pub fn init_logging<App>(element_id: impl Into<String>) -> Result<(), log::SetLoggerError>
where
    App: ApplicationSpecific + 'static,
{
    let logger = Box::new(DomConsoleLogger {
        // Make consistent, and configurable.
        level: LevelFilter::Info,
        // TODO: make the ID configurable.
        element_id: element_id.into(),
        log_lines: std::sync::Mutex::new(VecDeque::new()),
        _app: std::marker::PhantomData::<fn() -> App>,
    });

    log::set_boxed_logger(logger)?;
    // Make consistent, and configurable.
    log::set_max_level(LevelFilter::Info);
    console_log("Test of console log fallback");
    Ok(())
}
