//! SVG files people add to a page (ADR 0074), kept only as pictures: no
//! scripts, no handlers, no embedded documents and no links to run.

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};

/// Elements dropped with everything inside them.
const DROPPED: &[&str] = &["script", "foreignObject", "iframe", "object", "embed", "handler", "listener"];

/// `svg` with what could run removed: the elements in [`DROPPED`], every
/// `on…` attribute, and links that aren't to the file's own fragments or
/// to images. Fails when it isn't well-formed XML with an `svg` root.
pub fn sanitized_svg(svg: &[u8]) -> Result<Vec<u8>, String> {
    let mut reader = Reader::from_reader(svg);
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Vec::new());
    let mut skipping = 0usize;
    let mut root = None;
    let mut buffer = Vec::new();
    loop {
        let event = reader.read_event_into(&mut buffer).map_err(|error| format!("the SVG isn't well-formed: {error}"))?;
        match event {
            Event::Eof => break,
            Event::Start(start) => {
                if skipping > 0 || DROPPED.contains(&local(&start)) {
                    skipping += 1;
                } else {
                    root.get_or_insert_with(|| local(&start).to_owned());
                    writer.write_event(Event::Start(cleaned(&start)?)).map_err(|error| error.to_string())?;
                }
            }
            Event::End(end) => {
                if skipping > 0 {
                    skipping -= 1;
                } else {
                    writer.write_event(Event::End(end)).map_err(|error| error.to_string())?;
                }
            }
            Event::Empty(empty) => {
                if skipping == 0 && !DROPPED.contains(&local(&empty)) {
                    root.get_or_insert_with(|| local(&empty).to_owned());
                    writer.write_event(Event::Empty(cleaned(&empty)?)).map_err(|error| error.to_string())?;
                }
            }
            // A processing instruction or a doctype can declare entities or
            // stylesheets; neither is kept.
            Event::PI(_) | Event::DocType(_) => {}
            other if skipping == 0 => writer.write_event(other).map_err(|error| error.to_string())?,
            _ => {}
        }
        buffer.clear();
    }
    if root.as_deref() != Some("svg") {
        return Err("that isn't an SVG".into());
    }
    Ok(writer.into_inner())
}

fn local<'a>(start: &'a BytesStart<'_>) -> &'a str {
    let name = start.name().into_inner();
    name.rsplit(':').next().unwrap_or(name)
}

/// `start` with its handlers and runnable links taken out.
fn cleaned(start: &BytesStart<'_>) -> Result<BytesStart<'static>, String> {
    let name = start.name().into_inner().to_owned();
    let mut kept = BytesStart::new(name);
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|error| format!("the SVG has a bad attribute: {error}"))?;
        let name = attribute.key.into_inner().to_owned();
        let key = name.to_ascii_lowercase();
        let value = attribute.normalized_value(quick_xml::XmlVersion::Implicit1_0).map_err(|error| error.to_string())?;
        let bare: String =
            value.chars().filter(|char| !char.is_whitespace() && !char.is_control()).collect::<String>().to_ascii_lowercase();
        if key.rsplit(':').next().is_some_and(|local| local.starts_with("on")) {
            continue;
        }
        let link = key == "href" || key.ends_with(":href") || key == "src";
        if link
            && !(bare.starts_with('#')
                || bare.starts_with("data:image/png")
                || bare.starts_with("data:image/jpeg")
                || bare.starts_with("data:image/gif")
                || bare.starts_with("data:image/webp"))
        {
            continue;
        }
        if bare.contains("javascript:") || (key == "style" && (bare.contains("url(") && !bare.contains("url(#"))) {
            continue;
        }
        kept.push_attribute((name.as_str(), value.as_ref()));
    }
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_svg_keeps_its_drawing_and_loses_what_could_run() {
        let svg = br##"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 10 10" onload="alert(1)"><script>alert(2)</script><defs><circle id="c" r="2"/></defs><use href="#c"/><a xlink:href="javascript:alert(3)"><rect width="5" height="5" fill="teal" onclick="x()"/></a><foreignObject><div>hi</div></foreignObject><image href="https://example.com/x.png"/></svg>"##;
        let kept = String::from_utf8(sanitized_svg(svg).expect("an svg")).expect("utf-8");
        assert!(
            kept.contains(r#"viewBox="0 0 10 10""#) && kept.contains(r#"fill="teal""#) && kept.contains(r##"href="#c""##),
            "{kept}"
        );
        for gone in ["onload", "onclick", "<script", "alert", "foreignObject", "javascript", "example.com"] {
            assert!(!kept.contains(gone), "{gone} stayed: {kept}");
        }
    }

    #[test]
    fn only_an_svg_is_one() {
        assert!(sanitized_svg(b"<html><body/></html>").is_err());
        assert!(sanitized_svg(b"<svg><g></svg>").is_err(), "not well-formed");
    }
}
