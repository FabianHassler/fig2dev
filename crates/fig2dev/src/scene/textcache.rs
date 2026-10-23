//! Cache of shaped text, keyed by font, size and string.

use super::TextPrim;
use figbase::text::{Metrics, Shaped};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const MAX_ENTRIES: usize = 20_000;

type Key = (i32, bool, u64, String, Option<String>);

thread_local! {
    static CACHE: RefCell<HashMap<Key, Rc<Shaped>>> = RefCell::new(HashMap::new());
}

/// Shaped outlines for a text primitive (cached).
pub(crate) fn shaped(t: &TextPrim) -> Rc<Shaped> {
    let key: Key = (
        t.font,
        t.postscript,
        t.size.to_bits(),
        t.text.clone(),
        t.local_font.clone(),
    );
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(s) = c.get(&key) {
            return s.clone();
        }
        if c.len() >= MAX_ENTRIES {
            c.clear();
        }
        let s = Rc::new(match t.local() {
            Some(lf) => lf.shape(t.size, &t.text),
            None => figbase::text::shape(&t.request(), t.size, &t.text),
        });
        c.insert(key, s.clone());
        s
    })
}

/// Metrics for a text primitive.
pub(crate) fn metrics(t: &TextPrim) -> Metrics {
    shaped(t).metrics
}
