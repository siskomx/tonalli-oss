use std::cell::{Cell, RefCell};

use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};

use crate::pack::PackError;

/// Reuse the index's UTF-8 plain-text/HTML extraction for bounded retrieved passages.
pub fn article_text(mime: &str, bytes: &[u8]) -> Result<String, PackError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| PackError::Corrupt("article text is not UTF-8".into()))?;
    if mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case("text/plain")
    {
        return Ok(text.into());
    }
    let tokenizer = Tokenizer::new(TextSink::default(), TokenizerOpts::default());
    let input = BufferQueue::default();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + 8192).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        input.push_back(text[start..end].into());
        let _ = tokenizer.feed(&input);
        start = end;
    }
    tokenizer.end();
    let mut text = tokenizer.sink.text.into_inner();
    if text.ends_with(' ') {
        text.pop();
    }
    Ok(text)
}

#[derive(Default)]
struct TextSink {
    text: RefCell<String>,
    skipped: Cell<Option<&'static str>>,
}

impl TokenSink for TextSink {
    type Handle = ();
    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        match token {
            Token::CharacterTokens(text) if self.skipped.get().is_none() => {
                let mut output = self.text.borrow_mut();
                for ch in text.chars() {
                    if ch.is_whitespace() {
                        separate(&mut output);
                    } else {
                        output.push(ch);
                    }
                }
            }
            Token::TagToken(tag) => {
                let name = tag.name.as_ref();
                if tag.kind == TagKind::StartTag && matches!(name, "script" | "style") {
                    self.skipped
                        .set(Some(if name == "script" { "script" } else { "style" }));
                    separate(&mut self.text.borrow_mut());
                    return TokenSinkResult::RawData(if name == "script" {
                        RawKind::ScriptData
                    } else {
                        RawKind::Rawtext
                    });
                }
                if tag.kind == TagKind::EndTag && self.skipped.get() == Some(name) {
                    self.skipped.set(None);
                }
                if matches!(
                    name,
                    "p" | "div"
                        | "section"
                        | "article"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "li"
                        | "ul"
                        | "ol"
                        | "br"
                        | "td"
                        | "tr"
                        | "table"
                        | "blockquote"
                        | "pre"
                        | "hr"
                        | "dt"
                        | "dd"
                        | "script"
                        | "style"
                ) {
                    separate(&mut self.text.borrow_mut());
                }
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

fn separate(output: &mut String) {
    if !output.is_empty() && !output.ends_with(' ') {
        output.push(' ');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extraction_decodes_entities_preserves_inline_words_and_excludes_code() {
        let html = br#"<p>A &amp; B</p><div>tre<b>foil</b> knot</div><script>if (a < b) { secret_token(); }</script><style>.secret_style{}</style><p>end&nbsp;here</p><!-- hidden_comment -->"#;
        assert_eq!(
            article_text("text/html", html).unwrap(),
            "A & B trefoil knot end here"
        );
    }
}
