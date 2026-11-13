//! Fetching the speech models. Every byte is checked against a pinned hash,
//! and the directory appears only once it's whole.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::VoiceError;

pub const ENCODER: &str = "encoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx";
pub const DECODER: &str = "decoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx";
pub const JOINER: &str = "joiner-epoch-99-avg-1-chunk-16-left-128.int8.onnx";
pub const TOKENS: &str = "tokens.txt";
pub const PUNCTUATION: &str = "model.int8.onnx";
pub const VOCABULARY: &str = "bpe.vocab";

const ZIPFORMER_URL: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-2023-06-26/resolve/main/";
const ZIPFORMER_FILES: [Download; 4] = [
    Download { name: ENCODER, sha256: "563fde436d16cf7607cf408cd6b30909819d03162652ef389c2450ced3f45ac1", size: 71_083_163 },
    Download { name: DECODER, sha256: "98da299f471e38bb4e1a8df579b8cc9122d6039576a77e357b3c60f17dd83b02", size: 1_307_236 },
    Download { name: JOINER, sha256: "d944208d660d67c8d72cd2acaeac971fa5ceb8c80e76c1968148846fedd6e297", size: 259_335 },
    Download { name: TOKENS, sha256: "49e3c2646595fd907228b3c6787069658f67b17377c60aeb8619c4551b2316fb", size: 5_048 },
];

const PUNCTUATION_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/punctuation-models/sherpa-onnx-online-punct-en-2024-08-06.tar.bz2";
const PUNCTUATION_ARCHIVE: Download = Download {
    name: "punctuation.tar.bz2",
    sha256: "9f5e5a72c7d2829635bd074fce92b6bbd5b78da8a52e7ad8ed1be933f366b99d",
    size: 30_667_839,
};

struct Download {
    name: &'static str,
    sha256: &'static str,
    size: u64,
}

fn total_bytes() -> u64 {
    ZIPFORMER_FILES.iter().map(|file| file.size).sum::<u64>() + PUNCTUATION_ARCHIVE.size
}

/// Copies `from` to `to`, which must end up exactly `size` bytes with the
/// hash `sha256`; a source sending more stops being read at once.
fn copy_checked(
    mut from: impl Read,
    to: &mut impl Write,
    file: &Download,
    on_bytes: &mut impl FnMut(u64),
) -> Result<(), VoiceError> {
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut seen = 0;
    loop {
        let read = from.read(&mut buffer).map_err(VoiceError::Fetch)?;
        if read == 0 {
            break;
        }
        seen += read as u64;
        if seen > file.size {
            return Err(VoiceError::Corrupt(file.name));
        }
        hash.update(&buffer[..read]);
        to.write_all(&buffer[..read]).map_err(VoiceError::Disk)?;
        on_bytes(read as u64);
    }
    let found: String = hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
    if seen == file.size && found == file.sha256 { Ok(()) } else { Err(VoiceError::Corrupt(file.name)) }
}

fn fetch(url: &str, file: &Download, dir: &Path, on_bytes: &mut impl FnMut(u64)) -> Result<(), VoiceError> {
    let response = ureq::get(url).call().map_err(|error| VoiceError::Request(error.to_string()))?;
    let mut out = File::create(dir.join(file.name)).map_err(VoiceError::Disk)?;
    copy_checked(response.into_body().into_reader(), &mut out, file, on_bytes)
}

/// Takes `wanted` out of the archive at `archive`, whose hash has been
/// checked, and removes it.
fn unpack(archive: &Path, wanted: [&str; 2], dir: &Path) -> Result<(), VoiceError> {
    let reader = File::open(archive).map_err(VoiceError::Disk)?;
    let mut tar = tar::Archive::new(bzip2::read::BzDecoder::new(reader));
    let mut found = 0;
    for entry in tar.entries().map_err(VoiceError::Disk)? {
        let mut entry = entry.map_err(VoiceError::Disk)?;
        let name = entry.path().map_err(VoiceError::Disk)?.file_name().map(|name| name.to_string_lossy().into_owned());
        if let Some(name) = name.filter(|name| wanted.contains(&name.as_str())) {
            let mut out = File::create(dir.join(name)).map_err(VoiceError::Disk)?;
            io::copy(&mut entry, &mut out).map_err(VoiceError::Disk)?;
            found += 1;
        }
    }
    if found != wanted.len() {
        return Err(VoiceError::Corrupt(PUNCTUATION_ARCHIVE.name));
    }
    fs::remove_file(archive).map_err(VoiceError::Disk)
}

/// Downloads the models to the directory `to`, reporting bytes so far and in
/// all. A directory already there is whole, as only a finished one is moved.
pub fn download(to: &Path, mut report: impl FnMut(u64, u64)) -> Result<(), VoiceError> {
    if to.is_dir() {
        return Ok(());
    }
    let partial = to.with_extension("partial");
    if partial.exists() {
        fs::remove_dir_all(&partial).map_err(VoiceError::Disk)?;
    }
    fs::create_dir_all(&partial).map_err(VoiceError::Disk)?;
    let total = total_bytes();
    let mut done = 0;
    let mut on_bytes = |bytes| {
        done += bytes;
        report(done, total);
    };
    for file in &ZIPFORMER_FILES {
        fetch(&format!("{ZIPFORMER_URL}{}", file.name), file, &partial, &mut on_bytes)?;
    }
    fetch(PUNCTUATION_URL, &PUNCTUATION_ARCHIVE, &partial, &mut on_bytes)?;
    unpack(&partial.join(PUNCTUATION_ARCHIVE.name), [PUNCTUATION, VOCABULARY], &partial)?;
    fs::rename(&partial, to).map_err(VoiceError::Disk)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO: Download =
        Download { name: "hello", sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824", size: 5 };

    fn copy(bytes: &[u8], file: &Download) -> (Result<(), VoiceError>, Vec<u8>) {
        let mut out = Vec::new();
        let result = copy_checked(bytes, &mut out, file, &mut |_| {});
        (result, out)
    }

    #[test]
    fn the_right_bytes_are_copied() {
        let (result, out) = copy(b"hello", &HELLO);
        assert!(result.is_ok());
        assert_eq!(out, b"hello");
    }

    #[test]
    fn bytes_with_the_wrong_hash_are_refused() {
        assert!(matches!(copy(b"jello", &HELLO).0, Err(VoiceError::Corrupt("hello"))));
    }

    #[test]
    fn too_few_or_too_many_bytes_are_refused() {
        assert!(matches!(copy(b"hell", &HELLO).0, Err(VoiceError::Corrupt("hello"))));
        let (result, out) = copy(b"hello, and on and on", &HELLO);
        assert!(matches!(result, Err(VoiceError::Corrupt("hello"))));
        assert!(out.is_empty(), "nothing past the limit is written");
    }

    #[test]
    fn a_directory_already_there_is_left_alone() {
        let dir = tempfile::tempdir().expect("a directory");
        let mut asked = false;
        assert!(download(dir.path(), |_, _| asked = true).is_ok());
        assert!(!asked);
    }

    #[test]
    fn the_total_counts_every_file() {
        assert_eq!(total_bytes(), 72_654_782 + 30_667_839);
    }
}
