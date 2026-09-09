use std::error::Error;

// Dependency Display strings can contain URIs, header values or HTTP/2 debug data.
// Emit only exact static messages from the pinned transports and typed IO metadata.
const SAFE_MESSAGES: &[&str] = &[
    "client error (Canceled)",
    "client error (ChannelClosed)",
    "client error (Connect)",
    "client error (ProxyConnect)",
    "client error (UserUnsupportedRequestMethod)",
    "client error (UserUnsupportedVersion)",
    "client error (UserAbsoluteUriRequired)",
    "client error (SendRequest)",
    "connection closed before message completed",
    "received unexpected message from connection",
    "channel closed",
    "operation was canceled",
    "operation timed out",
    "error reading a body from connection",
    "error writing a body to connection",
    "error shutting down connection",
    "http2 error",
    "connection error",
    "dispatch task is gone",
    "invalid HTTP header parsed",
    "invalid content-length parsed",
    "unexpected transfer-encoding parsed",
    "message head is too large",
    "invalid HTTP status-code parsed",
    "user error: inactive stream",
    "user error: unexpected frame type",
    "user error: payload too big",
    "user error: rejected",
    "user error: release capacity too big",
    "user error: stream ID overflowed",
    "user error: malformed headers",
    "user error: request URI missing scheme and authority",
];

pub(super) fn source_summary(error: &(dyn Error + 'static)) -> String {
    let mut labels = Vec::new();
    let mut source = error.source();
    for _ in 0..12 {
        let Some(cause) = source else { break };
        let label = if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            format!("io(kind={:?},os={:?})", io.kind(), io.raw_os_error())
        } else {
            let text = cause.to_string();
            SAFE_MESSAGES.iter().copied().find(|safe| *safe == text)
                .unwrap_or("unclassified(redacted)").to_string()
        };
        labels.push(label);
        source = cause.source();
    }
    if source.is_some() {
        labels.push("truncated".to_string());
    }
    labels.join(" -> ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;

    #[derive(Debug)]
    struct Wrapped(&'static str, Option<Box<dyn Error>>);

    impl fmt::Display for Wrapped {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.0)
        }
    }

    impl Error for Wrapped {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            self.1.as_deref()
        }
    }

    #[test]
    fn source_summary_preserves_nested_io_kind_without_message() {
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "cookie=private");
        let error = Wrapped("https://user:password@host/?token=private", Some(Box::new(
            Wrapped("client error (SendRequest)", Some(Box::new(io)))
        )));
        assert_eq!(source_summary(&error),
            "client error (SendRequest) -> io(kind=ConnectionReset,os=None)");
    }

    #[test]
    fn source_summary_retains_static_protocol_cause() {
        let error = Wrapped("request", Some(Box::new(Wrapped("http2 error", Some(Box::new(
            Wrapped("user error: malformed headers", None)
        ))))));
        assert_eq!(source_summary(&error), "http2 error -> user error: malformed headers");
    }

    #[test]
    fn source_summary_redacts_unknown_and_suffixed_messages() {
        for text in ["Authorization: private", "user error: malformed headers cookie=private",
                     "connection error received: PROTOCOL_ERROR (private debug data)"] {
            let error = Wrapped("request", Some(Box::new(Wrapped(text, None))));
            assert_eq!(source_summary(&error), "unclassified(redacted)");
        }
    }

    #[test]
    fn source_summary_bounds_cycles_and_handles_no_source() {
        #[derive(Debug)]
        struct Cycle;
        impl fmt::Display for Cycle {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("channel closed") }
        }
        impl Error for Cycle {
            fn source(&self) -> Option<&(dyn Error + 'static)> { Some(self) }
        }
        assert_eq!(source_summary(&Cycle), format!("{} -> truncated", vec!["channel closed"; 12].join(" -> ")));
        assert_eq!(source_summary(&Wrapped("private", None)), "");
    }
}
