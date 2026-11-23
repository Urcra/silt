//! An insertion's characters, held as UTF-8 and indexed by character: a
//! `Vec<char>` took four bytes for each, most of a long family's memory
//! (ADR 0116).

use std::fmt;
use std::ops::Range;

use serde::de::{SeqAccess, Visitor};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Characters between two marks; looking one up walks at most this many.
const STRIDE: usize = 128;

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct Chars {
    text: String,
    len: usize,
    /// The byte offset of every `STRIDE`th character, unused while the text
    /// is ASCII, where a character's offset is its byte's.
    marks: Vec<u32>,
}

impl Chars {
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn push_str(&mut self, content: &str) {
        for ch in content.chars() {
            if self.len.is_multiple_of(STRIDE) {
                self.marks.push(u32::try_from(self.text.len()).expect("insertions are shorter than 4 GiB"));
            }
            self.text.push(ch);
            self.len += 1;
        }
    }

    /// The characters in `range`, counted in characters.
    pub(crate) fn slice(&self, range: Range<usize>) -> &str {
        &self.text[self.byte(range.start)..self.byte(range.end)]
    }

    fn byte(&self, index: usize) -> usize {
        if index == self.len {
            return self.text.len();
        }
        if self.text.len() == self.len {
            return index;
        }
        let mark = self.marks[index / STRIDE] as usize;
        let skip = index % STRIDE;
        self.text[mark..].char_indices().nth(skip).map_or(self.text.len(), |(at, _)| mark + at)
    }
}

impl From<&str> for Chars {
    fn from(content: &str) -> Self {
        let mut chars = Self::default();
        chars.push_str(content);
        chars
    }
}

impl fmt::Debug for Chars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.text, f)
    }
}

/// Encoded as the `Vec<char>` it replaced, so snapshots keep their format.
impl Serialize for Chars {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.len))?;
        for ch in self.text.chars() {
            seq.serialize_element(&ch)?;
        }
        seq.end()
    }
}

impl<'de> Deserialize<'de> for Chars {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct CharsVisitor;

        impl<'de> Visitor<'de> for CharsVisitor {
            type Value = Chars;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a sequence of characters")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Chars, A::Error> {
                let mut chars = Chars::default();
                let mut buffer = [0; 4];
                while let Some(ch) = seq.next_element::<char>()? {
                    chars.push_str(ch.encode_utf8(&mut buffer));
                }
                Ok(chars)
            }
        }

        deserializer.deserialize_seq(CharsVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sliced(content: &str, range: Range<usize>) -> String {
        content.chars().skip(range.start).take(range.len()).collect()
    }

    #[test]
    fn slices_count_characters_in_ascii_and_wider_text() {
        let wide: String = (0..1000).map(|i| ['a', 'é', '漢', '🦀'][i % 4]).collect();
        for content in ["", "plain ascii text", wide.as_str()] {
            let chars = Chars::from(content);
            let len = content.chars().count();
            assert_eq!(chars.len(), len);
            for range in [0..0, 0..len, 0..len / 2, len / 3..len, len / 4..len / 2] {
                assert_eq!(chars.slice(range.clone()), sliced(content, range));
            }
        }
    }

    #[test]
    fn text_that_turns_wide_part_way_is_still_found() {
        let mut chars = Chars::from("a".repeat(300).as_str());
        chars.push_str("é漢");
        chars.push_str(&"b".repeat(200));
        assert_eq!(chars.len(), 502);
        assert_eq!(chars.slice(299..303), "aé漢b");
        assert_eq!(chars.slice(500..502), "bb");
    }

    #[test]
    fn encodes_as_the_vec_of_chars_it_replaced() {
        let content = "héllo 漢字 🦀";
        let old: Vec<char> = content.chars().collect();
        let encoded = postcard::to_allocvec(&Chars::from(content)).expect("encodes");
        assert_eq!(encoded, postcard::to_allocvec(&old).expect("encodes"));
        let decoded: Chars = postcard::from_bytes(&encoded).expect("decodes");
        assert_eq!(decoded, Chars::from(content));
    }
}
