//! A prompt with files attached, as the content blocks the agent says it
//! takes (ACP's prompt capabilities).

use agent_client_protocol::schema::v1::{
    BlobResourceContents, ContentBlock, EmbeddedResource, EmbeddedResourceResource, ImageContent, TextContent,
    TextResourceContents,
};
use agent_client_protocol::schema::v2;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub data: Vec<u8>,
}

impl Attachment {
    fn is_image(&self) -> bool {
        self.mime.starts_with("image/")
    }

    fn text(&self) -> Option<&str> {
        std::str::from_utf8(&self.data).ok()
    }

    fn uri(&self) -> String {
        format!("attachment:{}", self.name)
    }
}

/// What the agent takes in a prompt besides text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Accepts {
    pub image: bool,
    /// Files' contents as resources of their own.
    pub embedded: bool,
}

impl Accepts {
    /// Whether the agent can be given `attachment`: an image only as one,
    /// and any text, embedded or written into the prompt; other files only
    /// embedded.
    pub fn takes(self, attachment: &Attachment) -> bool {
        if attachment.is_image() { self.image } else { self.embedded || attachment.text().is_some() }
    }
}

/// The prompt's blocks: its text, with text files written into it for an
/// agent that takes no resources, then the images and resources. Files the
/// agent doesn't take are left out; [`Accepts::takes`] says which.
pub(crate) fn blocks(text: &str, attachments: &[Attachment], accepts: Accepts) -> Vec<ContentBlock> {
    let mut said = text.to_owned();
    let mut rest = Vec::new();
    for attachment in attachments.iter().filter(|attachment| accepts.takes(attachment)) {
        if attachment.is_image() {
            rest.push(ContentBlock::Image(ImageContent::new(BASE64.encode(&attachment.data), attachment.mime.clone())));
            continue;
        }
        let resource = match attachment.text() {
            Some(text) if !accepts.embedded => {
                said.push_str(&format!("\n\n{}:\n```\n{text}\n```", attachment.name));
                continue;
            }
            Some(text) => EmbeddedResourceResource::TextResourceContents(
                TextResourceContents::new(text, attachment.uri()).mime_type(attachment.mime.clone()),
            ),
            None => EmbeddedResourceResource::BlobResourceContents(
                BlobResourceContents::new(BASE64.encode(&attachment.data), attachment.uri()).mime_type(attachment.mime.clone()),
            ),
        };
        rest.push(ContentBlock::Resource(EmbeddedResource::new(resource)));
    }
    let mut blocks = vec![ContentBlock::Text(TextContent::new(said))];
    blocks.extend(rest);
    blocks
}

/// [`blocks`] for an agent speaking v2, whose text, images and resources
/// read as v1's.
pub(crate) fn blocks_v2(
    text: &str,
    attachments: &[Attachment],
    accepts: Accepts,
) -> Result<Vec<v2::ContentBlock>, serde_json::Error> {
    blocks(text, attachments, accepts).into_iter().map(|block| serde_json::from_value(serde_json::to_value(block)?)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(name: &str, mime: &str, data: &[u8]) -> Attachment {
        Attachment { name: name.into(), mime: mime.into(), data: data.to_vec() }
    }

    #[test]
    fn files_go_as_the_agent_takes_them() {
        let image = attachment("shot.png", "image/png", &[0x89, b'P', b'N', b'G']);
        let notes = attachment("notes.txt", "text/plain", b"remember this");
        let binary = attachment("data.bin", "application/octet-stream", &[0xff, 0xfe]);

        let plain = Accepts::default();
        assert!(!plain.takes(&image));
        assert!(plain.takes(&notes));
        assert!(!plain.takes(&binary));
        let inline = blocks("look", std::slice::from_ref(&notes), plain);
        assert_eq!(inline.len(), 1);
        let ContentBlock::Text(text) = &inline[0] else { panic!("text first") };
        assert_eq!(text.text, "look\n\nnotes.txt:\n```\nremember this\n```");

        let all = Accepts { image: true, embedded: true };
        let given = blocks("look", &[image, notes, binary], all);
        assert!(matches!(&given[0], ContentBlock::Text(text) if text.text == "look"));
        assert!(matches!(&given[1], ContentBlock::Image(image) if image.mime_type == "image/png"));
        assert!(matches!(
            &given[2],
            ContentBlock::Resource(EmbeddedResource { resource: EmbeddedResourceResource::TextResourceContents(text), .. })
                if text.uri == "attachment:notes.txt"
        ));
        assert!(matches!(
            &given[3],
            ContentBlock::Resource(EmbeddedResource { resource: EmbeddedResourceResource::BlobResourceContents(_), .. })
        ));
    }

    #[test]
    fn v2_takes_the_same_blocks() {
        let files = [
            attachment("shot.png", "image/png", &[0x89, b'P', b'N', b'G']),
            attachment("notes.txt", "text/plain", b"remember this"),
            attachment("data.bin", "application/octet-stream", &[0xff, 0xfe]),
        ];
        let all = Accepts { image: true, embedded: true };
        let v1: Vec<serde_json::Value> =
            blocks("look", &files, all).iter().map(|block| serde_json::to_value(block).expect("json")).collect();
        let v2: Vec<serde_json::Value> = blocks_v2("look", &files, all)
            .expect("v2 blocks")
            .iter()
            .map(|block| serde_json::to_value(block).expect("json"))
            .collect();
        assert_eq!(v1, v2);
        assert!(matches!(blocks_v2("look", &files, all).expect("v2 blocks")[1], v2::ContentBlock::Image(_)));
    }
}
