//! What one dictation says as the recognizer's text changes, apart from the
//! engine that produces it.

use serde::Serialize;

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Event {
    /// The words of the current phrase so far; they may still change.
    Partial { text: String },
    /// A finished phrase, tidied.
    Segment { text: String },
    /// Silence went on past a finished phrase.
    Paused,
    /// All the audio has been heard.
    Done,
}

#[derive(Default)]
pub struct Take {
    shown: String,
    spoke: bool,
    paused: bool,
}

impl Take {
    /// The recognizer's `text` for the current phrase, and whether it has
    /// just ended the phrase; `tidy` finishes a phrase's text.
    pub fn step(&mut self, text: &str, endpoint: bool, tidy: impl Fn(&str) -> String) -> Vec<Event> {
        let text = text.trim();
        if endpoint {
            self.shown.clear();
            if !text.is_empty() {
                self.spoke = true;
                self.paused = false;
                return vec![Event::Segment { text: tidy(text) }];
            }
            if self.spoke && !self.paused {
                self.paused = true;
                return vec![Event::Paused];
            }
            return Vec::new();
        }
        if text.is_empty() || text == self.shown {
            return Vec::new();
        }
        text.clone_into(&mut self.shown);
        vec![Event::Partial { text: text.to_lowercase() }]
    }

    /// The end of the audio, with whatever the recognizer still holds.
    pub fn finish(self, text: &str, tidy: impl Fn(&str) -> String) -> Vec<Event> {
        let text = text.trim();
        let mut events = Vec::new();
        if !text.is_empty() {
            events.push(Event::Segment { text: tidy(text) });
        }
        events.push(Event::Done);
        events
    }
}

/// 32-bit float samples from bytes that may arrive split at any point.
#[derive(Default)]
pub struct Samples {
    rest: Vec<u8>,
}

impl Samples {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<f32> {
        self.rest.extend_from_slice(bytes);
        let whole = self.rest.len() - self.rest.len() % 4;
        let samples = self.rest[..whole].as_chunks::<4>().0.iter().map(|sample| f32::from_le_bytes(*sample)).collect();
        self.rest.drain(..whole);
        samples
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tidy(text: &str) -> String {
        format!("<{text}>")
    }

    fn partial(text: &str) -> Event {
        Event::Partial { text: text.to_owned() }
    }

    fn segment(text: &str) -> Event {
        Event::Segment { text: text.to_owned() }
    }

    #[test]
    fn words_show_lowered_and_only_when_they_change() {
        let mut take = Take::default();
        assert_eq!(take.step("HELLO", false, tidy), [partial("hello")]);
        assert_eq!(take.step("HELLO", false, tidy), []);
        assert_eq!(take.step("HELLO THERE", false, tidy), [partial("hello there")]);
    }

    #[test]
    fn a_phrase_ends_tidied() {
        let mut take = Take::default();
        take.step("HELLO", false, tidy);
        assert_eq!(take.step(" HELLO ", true, tidy), [segment("<HELLO>")]);
    }

    #[test]
    fn silence_before_any_speech_is_no_pause() {
        let mut take = Take::default();
        assert_eq!(take.step("", true, tidy), []);
        assert_eq!(take.step("", true, tidy), []);
    }

    #[test]
    fn silence_after_a_phrase_pauses_once_until_more_speech() {
        let mut take = Take::default();
        take.step("ONE", true, tidy);
        assert_eq!(take.step("", true, tidy), [Event::Paused]);
        assert_eq!(take.step("", true, tidy), []);
        take.step("TWO", true, tidy);
        assert_eq!(take.step("", true, tidy), [Event::Paused]);
    }

    #[test]
    fn the_end_keeps_the_words_still_held() {
        assert_eq!(Take::default().finish("LAST", tidy), [segment("<LAST>"), Event::Done]);
        assert_eq!(Take::default().finish(" ", tidy), [Event::Done]);
    }

    #[test]
    fn samples_survive_being_split_mid_value() {
        let bytes: Vec<u8> = [0.5f32, -0.25].iter().flat_map(|sample| sample.to_le_bytes()).collect();
        let mut samples = Samples::default();
        assert_eq!(samples.push(&bytes[..3]), Vec::<f32>::new());
        assert_eq!(samples.push(&bytes[3..6]), [0.5]);
        assert_eq!(samples.push(&bytes[6..]), [-0.25]);
    }

    #[test]
    fn events_are_written_as_the_daemon_reads_them() {
        assert_eq!(serde_json::to_string(&partial("a")).expect("json"), r#"{"kind":"partial","text":"a"}"#);
        assert_eq!(serde_json::to_string(&Event::Paused).expect("json"), r#"{"kind":"paused"}"#);
    }
}
