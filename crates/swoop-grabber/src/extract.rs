//! Streaming link extraction with `lol_html`.

use lol_html::{element, HtmlRewriter, Settings};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extracted {
    pub links: Vec<String>,
    pub media: Vec<String>,
    pub iframes: Vec<String>,
    pub base_href: Option<String>,
    pub title: Option<String>,
}

/// Extract candidate URLs from an HTML document without building a DOM.
pub fn extract(html: &[u8]) -> Extracted {
    let out = Rc::new(RefCell::new(Extracted::default()));
    let mk = |attr: &'static str, bucket: fn(&mut Extracted) -> &mut Vec<String>| {
        let out = out.clone();
        move |el: &mut lol_html::html_content::Element| {
            if let Some(v) = el.get_attribute(attr) {
                bucket(&mut out.borrow_mut()).push(v);
            }
            Ok(())
        }
    };
    let out_base = out.clone();
    let out_srcset = out.clone();
    let out_title = out.clone();
    let mut rewriter = HtmlRewriter::new(
        Settings {
            element_content_handlers: vec![
                element!("a[href]", mk("href", |e| &mut e.links)),
                element!("area[href]", mk("href", |e| &mut e.links)),
                element!("link[href]", mk("href", |e| &mut e.links)),
                element!("img[src]", mk("src", |e| &mut e.media)),
                element!("video[src]", mk("src", |e| &mut e.media)),
                element!("audio[src]", mk("src", |e| &mut e.media)),
                element!("source[src]", mk("src", |e| &mut e.media)),
                element!("embed[src]", mk("src", |e| &mut e.media)),
                element!("object[data]", mk("data", |e| &mut e.media)),
                element!("iframe[src]", mk("src", |e| &mut e.iframes)),
                element!("img[srcset], source[srcset]", move |el| {
                    if let Some(v) = el.get_attribute("srcset") {
                        for cand in v.split(',') {
                            if let Some(u) = cand.split_whitespace().next() {
                                out_srcset.borrow_mut().media.push(u.to_owned());
                            }
                        }
                    }
                    Ok(())
                }),
                element!("base[href]", move |el| {
                    if let Some(v) = el.get_attribute("href") {
                        out_base.borrow_mut().base_href = Some(v);
                    }
                    Ok(())
                }),
                lol_html::text!("title", move |t| {
                    let mut o = out_title.borrow_mut();
                    let s = o.title.get_or_insert_with(String::new);
                    s.push_str(t.as_str());
                    Ok(())
                }),
            ],
            ..Settings::default()
        },
        |_: &[u8]| {},
    );
    let _ = rewriter.write(html);
    let _ = rewriter.end();
    let mut result = out.borrow().clone();
    if let Some(t) = &mut result.title {
        *t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_links_and_media() {
        let html = br#"<html><head><title> Files  </title><base href="/base/"></head><body><a href="a.pdf">A</a><img src="i.png" srcset="i2.png 2x, i3.png 3x"><video><source src="v.mp4"></video><iframe src="f.html"></iframe></body></html>"#;
        let e = extract(html);
        assert_eq!(e.links, vec!["a.pdf"]);
        assert_eq!(e.media, vec!["i.png", "i2.png", "i3.png", "v.mp4"]);
        assert_eq!(e.iframes, vec!["f.html"]);
        assert_eq!(e.base_href.as_deref(), Some("/base/"));
        assert_eq!(e.title.as_deref(), Some("Files"));
    }
}
