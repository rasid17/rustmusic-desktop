//! XML helpers shared across the DLNA server (DIDL builder, SOAP envelope,
//! body parsing).

/// Escape `<`, `>`, `&` for XML element text content.
pub fn xml_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape characters for XML attribute values (also escapes `"`).
pub fn xml_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Extract the trimmed, entity-decoded text content of `<Tag>...</Tag>` (handles
/// XML namespace prefixes, element attributes, CDATA, comments, and case fallback).
pub fn extract_tag(body: &str, tag: &str) -> Option<String> {
    if let Some(exact) = crate::helper::xml::text(body, tag) {
        return Some(exact);
    }
    // Case-insensitive local-name fallback for quirkier UPnP/DLNA controllers.
    let target = tag.to_ascii_lowercase();
    let mut pos = 0;
    while let Some(rel) = body[pos..].find('<') {
        let open_start = pos + rel;
        let rest = &body[open_start..];
        if rest.starts_with("<!--") {
            pos = rest.find("-->").map_or(body.len(), |i| open_start + i + 3);
            continue;
        }
        let Some(gt) = rest.find('>') else { break };
        let inner = &rest[1..gt];
        if !inner.starts_with('/') && !inner.starts_with('?') && !inner.starts_with('!') && !inner.ends_with('/') {
            let name = inner.split(|c: char| c.is_whitespace()).next().unwrap_or("");
            let local = name.rsplit(':').next().unwrap_or(name);
            if local.eq_ignore_ascii_case(&target) {
                let content_start = open_start + gt + 1;
                let mut scan = content_start;
                while let Some(crel) = body[scan..].find("</") {
                    let close_start = scan + crel;
                    if let Some(cgt) = body[close_start..].find('>') {
                        let cname = body[close_start + 2..close_start + cgt].trim();
                        let clocal = cname.rsplit(':').next().unwrap_or(cname);
                        if clocal.eq_ignore_ascii_case(&target) {
                            return Some(crate::helper::xml::decode(&body[content_start..close_start]).trim().to_string());
                        }
                        scan = close_start + cgt + 1;
                    } else {
                        break;
                    }
                }
            }
        }
        pos = open_start + gt + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_tag_handles_namespaces_attributes_and_entities() {
        let soap = r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
            <s:Body><!-- comment <ObjectID>99</ObjectID> -->
                <u:Browse xmlns:u="urn:schemas-upnp-org:service:ContentDirectory:1">
                    <u:ObjectID dt="string">albums/rock&amp;roll</u:ObjectID>
                    <BrowseFlag>BrowseDirectChildren</BrowseFlag>
                    <startingIndex>1</startingIndex>
                </u:Browse>
            </s:Body></s:Envelope>"#;
        assert_eq!(extract_tag(soap, "ObjectID").as_deref(), Some("albums/rock&roll"));
        assert_eq!(extract_tag(soap, "BrowseFlag").as_deref(), Some("BrowseDirectChildren"));
        assert_eq!(extract_tag(soap, "StartingIndex").as_deref(), Some("1"));
    }
}
