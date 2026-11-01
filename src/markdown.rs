use pulldown_cmark::{html, Options, Parser};

pub fn to_html(markdown: &str) -> String {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;
    let mut html_out = String::with_capacity(markdown.len() * 2);
    html::push_html(&mut html_out, Parser::new_ext(markdown, opts));
    html_out
}
