//! XML 1.0 external entity TextDecl, distinct from document XMLDecl.
use crate::xml::{XmlError, XmlErrorKind};
#[derive(Debug, PartialEq, Eq)]
pub(super) struct TextDeclaration {
    pub version: Option<String>,
    pub encoding: String,
    pub body_start: usize,
}
struct Reader<'a> {
    text: &'a str,
    offset: usize,
}
impl Reader<'_> {
    fn rest(&self) -> &str {
        &self.text[self.offset..]
    }
    fn error(&self) -> XmlError {
        XmlError {
            offset: self.offset,
            kind: XmlErrorKind::Malformed,
            message: "invalid external XML text declaration".into(),
        }
    }
    fn take(&mut self, value: &str) -> bool {
        if self.rest().starts_with(value) {
            self.offset += value.len();
            true
        } else {
            false
        }
    }
    fn space(&mut self) -> bool {
        let start = self.offset;
        while self
            .rest()
            .as_bytes()
            .first()
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        {
            self.offset += 1;
        }
        start != self.offset
    }
    fn value(&mut self) -> Result<String, XmlError> {
        self.space();
        if !self.take("=") {
            return Err(self.error());
        }
        self.space();
        let quote = *self
            .rest()
            .as_bytes()
            .first()
            .filter(|b| matches!(b, b'\'' | b'"'))
            .ok_or_else(|| self.error())?;
        self.offset += 1;
        let end = self
            .rest()
            .find(quote as char)
            .ok_or_else(|| self.error())?;
        let value = self.rest()[..end].to_owned();
        self.offset += end + 1;
        Ok(value)
    }
}
pub(super) fn parse(text: &str) -> Result<Option<TextDeclaration>, XmlError> {
    if !text.starts_with("<?xml")
        || !text
            .as_bytes()
            .get(5)
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
    {
        return Ok(None);
    }
    let mut r = Reader { text, offset: 5 };
    r.space();
    let version = if r.take("version") {
        let value = r.value()?;
        if !value
            .strip_prefix("1.")
            .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(r.error());
        }
        if !r.space() {
            return Err(r.error());
        }
        Some(value)
    } else {
        None
    };
    if !r.take("encoding") {
        return Err(r.error());
    }
    let encoding = r.value()?;
    if !encoding
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphabetic)
        || !encoding
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(r.error());
    }
    r.space();
    if !r.take("?>") {
        return Err(r.error());
    }
    Ok(Some(TextDeclaration {
        version,
        encoding,
        body_start: r.offset,
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn whole_text_declaration_grammar_and_literal_body_boundary() {
        for decl in [
            "<?xml encoding='UTF-8'?>",
            "<?xml version='1.0' encoding=\"UTF-16\" ?>",
            "<?xml\nencoding = 'windows-1252'\r?>",
        ] {
            let source = format!("{decl}<root/>");
            let parsed = parse(&source).unwrap().unwrap();
            assert_eq!(&source[parsed.body_start..], "<root/>");
        }
        for source in [
            "<?xml version='1.0'?>",
            "<?xml standalone='no' encoding='utf-8'?>",
            "<?xml encoding='utf-8' standalone='no'?>",
            "<?xml encoding='utf-8' version='1.0'?>",
            "<?xml encoding='utf-8' encoding='utf-8'?>",
            "<?xml encoding='9bad'?>",
            "<?xml version='1' encoding='utf-8'?>",
            "<?xml encoding='utf 8'?>",
            "<?xml encoding='utf-8'>",
            "<?xml encoding='utf-8",
        ] {
            assert!(parse(source).is_err(), "{source}");
        }
        for source in [
            "<root/>",
            "<?xml-stylesheet href='x'?>",
            " <?xml encoding='utf-8'?>",
        ] {
            assert_eq!(parse(source).unwrap(), None);
        }
    }
}
