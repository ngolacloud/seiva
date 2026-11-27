//! Server-sent events and the assembly of a streamed answer.
//!
//! [`SseParser`] turns body chunks into events, whatever the chunk boundaries
//! (an event, a line or a UTF-8 character split across reads). A codec maps each
//! event to [`Piece`]s, and [`Assembler`] turns pieces into [`StreamEvent`]s and,
//! at the end, into the whole [`ChatResponse`] — the same assembly for every
//! provider.

use futures_util::StreamExt;
use serde_json::Value;
use std::collections::VecDeque;

use crate::stream::EventStream;
use crate::transport::BodyStream;
use crate::{ChatResponse, Content, LlmError, StopReason, StreamEvent, Usage};

/// The longest SSE line accepted (1 MiB): a provider cannot make the parser
/// buffer without bound.
const MAX_LINE: usize = 1024 * 1024;

/// One server-sent event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// An incremental SSE parser (the `event` and `data` fields; `id`, `retry` and
/// comments are ignored).
#[derive(Debug, Default)]
pub(crate) struct SseParser {
    buf: Vec<u8>,
    event: Option<String>,
    data: Option<String>,
}

impl SseParser {
    /// Feed a chunk; returns the events it completed.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, LlmError> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8(line)
                .map_err(|_| LlmError::Decode("the event stream is not UTF-8".into()))?;
            if let Some(ev) = self.line(&line) {
                out.push(ev);
            }
        }
        if self.buf.len() > MAX_LINE {
            return Err(LlmError::Decode(format!(
                "an event-stream line is longer than {MAX_LINE} bytes"
            )));
        }
        Ok(out)
    }

    /// The body ended: dispatch what is pending.
    pub fn finish(&mut self) -> Option<SseEvent> {
        if !self.buf.is_empty() {
            let rest = std::mem::take(&mut self.buf);
            if let Ok(line) = String::from_utf8(rest) {
                let line = line.trim_end_matches('\r').to_owned();
                if let Some(ev) = self.line(&line) {
                    return Some(ev);
                }
            }
        }
        self.dispatch()
    }

    fn line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_owned()),
            "data" => match &mut self.data {
                Some(d) => {
                    d.push('\n');
                    d.push_str(value);
                }
                None => self.data = Some(value.to_owned()),
            },
            _ => {}
        }
        None
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        let event = self.event.take();
        self.data.take().map(|data| SseEvent { event, data })
    }
}

/// What a codec reads out of one provider event.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Piece {
    /// The model that answers.
    Model(String),
    /// Text for content block `block` (the codec's own numbering).
    Text { block: usize, text: String },
    /// A tool call starts in block `block`.
    ToolStart {
        block: usize,
        id: String,
        name: String,
    },
    /// More arguments JSON for the tool call in block `block`.
    ToolArgs { block: usize, json: String },
    /// Usage so far. Providers report cumulative counts, so each field keeps
    /// its largest value.
    Usage(Usage),
    /// Why it stopped.
    Stop(StopReason),
    /// The provider's end-of-stream marker.
    Done,
}

enum Block {
    Text(String),
    Tool {
        id: String,
        name: String,
        json: String,
    },
}

/// Turns pieces into events and, at the end, into the whole answer.
#[derive(Default)]
pub(crate) struct Assembler {
    model: String,
    blocks: Vec<(usize, Block)>,
    usage: Usage,
    stop: Option<StopReason>,
    done: bool,
}

impl Assembler {
    fn position(&mut self, key: usize, new: impl FnOnce() -> Block) -> usize {
        if let Some(i) = self.blocks.iter().position(|(k, _)| *k == key) {
            return i;
        }
        self.blocks.push((key, new()));
        self.blocks.len() - 1
    }

    /// Feed a piece; returns the event to hand to the caller, if any.
    pub fn push(&mut self, piece: Piece) -> Option<StreamEvent> {
        match piece {
            Piece::Model(m) => {
                self.model = m;
                None
            }
            Piece::Text { block, text } => {
                if text.is_empty() {
                    return None;
                }
                let index = self.position(block, || Block::Text(String::new()));
                if let Block::Text(t) = &mut self.blocks[index].1 {
                    t.push_str(&text);
                }
                Some(StreamEvent::TextDelta { index, text })
            }
            Piece::ToolStart { block, id, name } => {
                let index = self.position(block, || Block::Tool {
                    id: id.clone(),
                    name: name.clone(),
                    json: String::new(),
                });
                Some(StreamEvent::ToolUseStart { index, id, name })
            }
            Piece::ToolArgs { block, json } => {
                if json.is_empty() {
                    return None;
                }
                let index = self.position(block, || Block::Tool {
                    id: String::new(),
                    name: String::new(),
                    json: String::new(),
                });
                if let Block::Tool { json: j, .. } = &mut self.blocks[index].1 {
                    j.push_str(&json);
                }
                Some(StreamEvent::ToolInputDelta {
                    index,
                    partial_json: json,
                })
            }
            Piece::Usage(u) => {
                self.usage.input_tokens = self.usage.input_tokens.max(u.input_tokens);
                self.usage.output_tokens = self.usage.output_tokens.max(u.output_tokens);
                self.usage.cache_read_tokens =
                    self.usage.cache_read_tokens.max(u.cache_read_tokens);
                self.usage.cache_write_tokens =
                    self.usage.cache_write_tokens.max(u.cache_write_tokens);
                None
            }
            Piece::Stop(reason) => {
                self.stop = Some(reason);
                None
            }
            Piece::Done => {
                self.done = true;
                None
            }
        }
    }

    /// Whether the provider said the stream is over.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// The whole answer. A stream that ended with neither an end marker nor a
    /// stop reason was cut: that is an error, never a partial answer passed off as
    /// complete.
    pub fn finish(self) -> Result<ChatResponse, LlmError> {
        let Some(stop) = self.stop.or(if self.done {
            Some(StopReason::EndTurn)
        } else {
            None
        }) else {
            return Err(LlmError::Transport(
                "the stream ended before the answer was complete".into(),
            ));
        };
        let content = self
            .blocks
            .into_iter()
            .map(|(_, b)| match b {
                Block::Text(text) => Content::Text { text },
                Block::Tool { id, name, json } => Content::ToolUse {
                    id,
                    name,
                    input: parse_arguments(&json),
                },
            })
            .collect();
        Ok(ChatResponse::new(self.model, content, stop, self.usage))
    }
}

/// Tool arguments as JSON: empty means `{}`; text that is not JSON is kept as a
/// JSON string for the caller's validation to reject.
pub(crate) fn parse_arguments(raw: &str) -> Value {
    if raw.trim().is_empty() {
        return Value::Object(serde_json::Map::new());
    }
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_owned()))
}

/// Drive a body through the SSE parser, a codec's event mapper and the assembler.
pub(crate) fn event_stream<F>(body: BodyStream, map: F) -> EventStream
where
    F: FnMut(SseEvent) -> Result<Vec<Piece>, LlmError> + Send + 'static,
{
    struct State<F> {
        body: BodyStream,
        parser: SseParser,
        assembler: Option<Assembler>,
        pending: VecDeque<Result<StreamEvent, LlmError>>,
        map: F,
    }

    impl<F> State<F>
    where
        F: FnMut(SseEvent) -> Result<Vec<Piece>, LlmError>,
    {
        fn feed(&mut self, events: Vec<SseEvent>) -> Result<(), LlmError> {
            for ev in events {
                for piece in (self.map)(ev)? {
                    if let Some(asm) = &mut self.assembler {
                        if let Some(out) = asm.push(piece) {
                            self.pending.push_back(Ok(out));
                        }
                    }
                }
            }
            Ok(())
        }

        fn complete(&mut self) {
            if let Some(asm) = self.assembler.take() {
                self.pending
                    .push_back(asm.finish().map(StreamEvent::Completed));
            }
        }
    }

    let state = State {
        body,
        parser: SseParser::default(),
        assembler: Some(Assembler::default()),
        pending: VecDeque::new(),
        map,
    };

    Box::pin(futures_util::stream::unfold(state, |mut st| async move {
        loop {
            if let Some(next) = st.pending.pop_front() {
                if next.is_err() {
                    st.assembler = None;
                    st.pending.clear();
                }
                return Some((next, st));
            }
            if st.assembler.as_ref().map_or(true, Assembler::is_done) {
                st.complete();
                if st.pending.is_empty() {
                    return None;
                }
                continue;
            }
            match st.body.next().await {
                Some(Ok(chunk)) => match st.parser.push(&chunk) {
                    Ok(events) => {
                        if let Err(e) = st.feed(events) {
                            st.pending.push_back(Err(e));
                        }
                    }
                    Err(e) => st.pending.push_back(Err(e)),
                },
                Some(Err(e)) => st.pending.push_back(Err(e)),
                None => {
                    let tail: Vec<SseEvent> = st.parser.finish().into_iter().collect();
                    if let Err(e) = st.feed(tail) {
                        st.pending.push_back(Err(e));
                    } else {
                        st.complete();
                    }
                    if st.pending.is_empty() {
                        return None;
                    }
                }
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(chunks: &[&[u8]]) -> Vec<SseEvent> {
        let mut p = SseParser::default();
        let mut out = Vec::new();
        for c in chunks {
            out.extend(p.push(c).unwrap());
        }
        out.extend(p.finish());
        out
    }

    #[test]
    fn events_survive_any_chunk_boundary() {
        let whole: &[u8] =
            "event: a\ndata: {\"x\":\"é\"}\n\n: ping\n\ndata: one\ndata: two\r\n\r\n".as_bytes();
        let expected = vec![
            SseEvent {
                event: Some("a".into()),
                data: "{\"x\":\"é\"}".into(),
            },
            SseEvent {
                event: None,
                data: "one\ntwo".into(),
            },
        ];
        assert_eq!(feed_all(&[whole]), expected);
        // Byte by byte, splitting the two-byte `é` too.
        let bytes: Vec<&[u8]> = whole.chunks(1).collect();
        assert_eq!(feed_all(&bytes), expected);
    }

    #[test]
    fn a_trailing_event_without_a_blank_line_is_dispatched_at_the_end() {
        assert_eq!(
            feed_all(&[b"data: [DONE]"]),
            vec![SseEvent {
                event: None,
                data: "[DONE]".into()
            }]
        );
    }

    #[test]
    fn an_endless_line_is_refused() {
        let mut p = SseParser::default();
        let big = vec![b'a'; MAX_LINE + 1];
        assert!(p.push(&big).is_err());
    }

    #[test]
    fn the_assembler_builds_text_and_tool_calls_in_order() {
        let mut a = Assembler::default();
        let pieces = vec![
            Piece::Model("m1".into()),
            Piece::Text {
                block: 0,
                text: "Hi ".into(),
            },
            Piece::ToolStart {
                block: 1,
                id: "t1".into(),
                name: "lookup".into(),
            },
            Piece::ToolArgs {
                block: 1,
                json: "{\"id\":".into(),
            },
            Piece::Text {
                block: 0,
                text: "there".into(),
            },
            Piece::ToolArgs {
                block: 1,
                json: "7}".into(),
            },
            Piece::Usage(Usage {
                input_tokens: 10,
                ..Usage::default()
            }),
            Piece::Usage(Usage {
                output_tokens: 4,
                ..Usage::default()
            }),
            Piece::Stop(StopReason::ToolUse),
        ];
        let events: Vec<_> = pieces.into_iter().filter_map(|p| a.push(p)).collect();
        assert_eq!(events.len(), 5);
        let r = a.finish().unwrap();
        assert_eq!(r.model, "m1");
        assert_eq!(r.text(), "Hi there");
        assert_eq!(r.tool_uses()[0].2, &serde_json::json!({"id": 7}));
        assert_eq!((r.usage.input_tokens, r.usage.output_tokens), (10, 4));
    }

    #[test]
    fn a_cut_stream_is_an_error_not_a_partial_answer() {
        let mut a = Assembler::default();
        a.push(Piece::Text {
            block: 0,
            text: "half".into(),
        });
        assert!(matches!(a.finish(), Err(LlmError::Transport(_))));
    }

    #[test]
    fn arguments_that_are_not_json_are_kept_as_a_string() {
        assert_eq!(parse_arguments(""), serde_json::json!({}));
        assert_eq!(parse_arguments("{oops"), Value::String("{oops".into()));
    }
}
