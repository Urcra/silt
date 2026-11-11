//! Live dictation: audio in, events out, as they happen.

use std::io::{ErrorKind, Read, Write};
use std::path::Path;

use sherpa_onnx::{
    OnlinePunctuation, OnlinePunctuationConfig, OnlinePunctuationModelConfig, OnlineRecognizer, OnlineRecognizerConfig,
    OnlineStream,
};

use crate::VoiceError;
use crate::model::{DECODER, ENCODER, JOINER, PUNCTUATION, TOKENS, VOCABULARY};
use crate::take::{Event, Samples, Take};

const RATE: i32 = 16_000;
const THREADS: i32 = 2;
/// A tenth of a second of audio at a time, so words show as they're said.
const READ_BYTES: usize = RATE as usize * 4 / 10;
/// Silence that lets the model finish the last word.
const TAIL_SECONDS: f32 = 0.66;
/// A phrase ends after this much silence…
const PHRASE_SILENCE: f32 = 1.2;
/// …and this much more makes a pause, about 2.5 seconds in all.
const PAUSE_SILENCE: f32 = 1.3;
const LONGEST_PHRASE: f32 = 20.0;

fn path_in(model: &Path, name: &str) -> Option<String> {
    Some(model.join(name).to_string_lossy().into_owned())
}

fn recognizer(model: &Path) -> Result<OnlineRecognizer, VoiceError> {
    let mut config = OnlineRecognizerConfig {
        decoding_method: Some("greedy_search".into()),
        enable_endpoint: true,
        rule1_min_trailing_silence: PAUSE_SILENCE,
        rule2_min_trailing_silence: PHRASE_SILENCE,
        rule3_min_utterance_length: LONGEST_PHRASE,
        ..Default::default()
    };
    config.model_config.transducer.encoder = path_in(model, ENCODER);
    config.model_config.transducer.decoder = path_in(model, DECODER);
    config.model_config.transducer.joiner = path_in(model, JOINER);
    config.model_config.tokens = path_in(model, TOKENS);
    config.model_config.num_threads = THREADS;
    OnlineRecognizer::create(&config).ok_or(VoiceError::Engine("the speech model"))
}

fn punctuator(model: &Path) -> Result<OnlinePunctuation, VoiceError> {
    let config = OnlinePunctuationConfig {
        model: OnlinePunctuationModelConfig {
            cnn_bilstm: path_in(model, PUNCTUATION),
            bpe_vocab: path_in(model, VOCABULARY),
            ..Default::default()
        },
    };
    OnlinePunctuation::create(&config).ok_or(VoiceError::Engine("the punctuation model"))
}

struct Listener<W> {
    recognizer: OnlineRecognizer,
    stream: OnlineStream,
    punctuator: OnlinePunctuation,
    output: W,
}

impl<W: Write> Listener<W> {
    fn tidy(&self, text: &str) -> String {
        let lowered = text.to_lowercase();
        self.punctuator.add_punctuation(&lowered).unwrap_or(lowered)
    }

    fn text(&self) -> String {
        while self.recognizer.is_ready(&self.stream) {
            self.recognizer.decode(&self.stream);
        }
        self.recognizer.get_result(&self.stream).map(|result| result.text).unwrap_or_default()
    }

    fn write(&mut self, events: Vec<Event>) -> Result<(), VoiceError> {
        for event in events {
            let line = serde_json::to_string(&event).expect("an event is plain data");
            writeln!(self.output, "{line}").map_err(VoiceError::Pipe)?;
        }
        self.output.flush().map_err(VoiceError::Pipe)
    }

    fn step(&mut self, take: &mut Take) -> Result<(), VoiceError> {
        let text = self.text();
        let endpoint = self.recognizer.is_endpoint(&self.stream);
        let events = take.step(&text, endpoint, |text| self.tidy(text));
        self.write(events)?;
        if endpoint {
            self.recognizer.reset(&self.stream);
        }
        Ok(())
    }
}

/// Reads 16 kHz mono `f32le` from `input` until it closes, writing one JSON
/// [`Event`] per line to `output`.
pub fn run(model: &Path, mut input: impl Read, output: impl Write) -> Result<(), VoiceError> {
    let recognizer = recognizer(model)?;
    let stream = recognizer.create_stream();
    let punctuator = punctuator(model)?;
    let mut listener = Listener { recognizer, stream, punctuator, output };
    let mut take = Take::default();
    let mut samples = Samples::default();
    let mut buffer = vec![0; READ_BYTES];
    loop {
        let read = match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(VoiceError::Pipe(error)),
        };
        listener.stream.accept_waveform(RATE, &samples.push(&buffer[..read]));
        listener.step(&mut take)?;
    }
    listener.stream.accept_waveform(RATE, &vec![0.0; (RATE as f32 * TAIL_SECONDS) as usize]);
    listener.stream.input_finished();
    let text = listener.text();
    let events = take.finish(&text, |text| listener.tidy(text));
    listener.write(events)
}
