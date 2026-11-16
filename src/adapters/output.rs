//! Terminal output for logs: by default `jamparty: [warning: ]message
//! key=value`, uncoloured, with control characters escaped whatever a
//! field holds; an explicit RUST_LOG gets fmt's detailed format.

use std::{
    fmt,
    io::{self, IsTerminal},
};

use tracing::{
    Event, Level, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{
    EnvFilter,
    fmt::{FmtContext, FormatEvent, FormatFields, format::Writer},
    registry::LookupSpan,
};

use crate::core::text::Untrusted;

/// Installs the global log subscriber, writing to stderr.
pub(crate) fn init() {
    let detailed = std::env::var_os("RUST_LOG").is_some();
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "jamparty=info".into());
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .with_ansi(io::stderr().is_terminal())
        .with_target(false);
    if detailed {
        subscriber.init();
    } else {
        subscriber.event_format(Compact).init();
    }
}

/// One line per event, whatever its fields hold.
struct Compact;

impl<S, N> FormatEvent<S, N> for Compact
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let mut fields = Fields::default();
        event.record(&mut fields);
        let level = match *event.metadata().level() {
            Level::ERROR => "error: ",
            Level::WARN => "warning: ",
            Level::INFO => "",
            Level::DEBUG => "debug: ",
            Level::TRACE => "trace: ",
        };
        writeln!(writer, "jamparty: {level}{}{}", fields.message, fields.rest)
    }
}

#[derive(Default)]
struct Fields {
    message: String,
    rest: String,
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        use fmt::Write;
        // Escaped here, not only where callers wrap text in `Untrusted`:
        // a `%` field or an interpolated message is written verbatim.
        let value = Untrusted(format_args!("{value:?}"));
        if field.name() == "message" {
            let _ = write!(self.message, "{value}");
        } else {
            let _ = write!(self.rest, " {}={value}", field.name());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        sync::{Arc, Mutex},
    };

    use crate::adapters::output::Compact;

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn compact_lines_have_messages_fields_and_severity() {
        let capture = Capture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .event_format(Compact)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(name = %"alice", fingerprint = %"abab", "joiner admitted");
            tracing::warn!("share is full");
            tracing::error!(attempts = 3, "delivery failed");
        });
        assert_eq!(
            String::from_utf8(capture.0.lock().unwrap().clone()).unwrap(),
            "jamparty: joiner admitted name=alice fingerprint=abab\njamparty: warning: share is full\njamparty: error: delivery failed attempts=3\n"
        );
    }

    /// Debug output that a hostile value fully controls.
    struct Raw(&'static str);

    impl std::fmt::Debug for Raw {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    #[test]
    fn control_characters_are_escaped_in_every_field() {
        const EVIL: &str = "a\r\njamparty: FORGED\u{1b}[2J\u{202e}b";
        let capture = Capture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .event_format(Compact)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(display = %EVIL, debug = ?Raw(EVIL), text = EVIL, "said {EVIL}");
        });
        let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert_eq!(output.lines().count(), 1, "{output:?}");
        assert!(
            !output
                .trim_end()
                .chars()
                .any(|c| c.is_control() || c == '\u{202e}'),
            "{output:?}"
        );
        assert!(
            output.contains(r"jamparty: said a\r\njamparty: FORGED\u{1b}[2J\u{202e}b display="),
            "{output:?}"
        );
    }
}
