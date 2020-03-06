use headers::{Header, HeaderName, HeaderValue};
use hyper::http::response::Builder;
use std::cmp::{max, min};
use std::ops::{Bound, RangeBounds};
use std::path::Path;

pub fn os_to_string(s: ::std::ffi::OsString) -> String {
    match s.into_string() {
        Ok(s) => s,
        Err(s) => {
            warn!("Invalid file name - cannot covert to UTF8 : {:?}", s);
            "INVALID_NAME".into()
        }
    }
}

pub fn parent_dir_exists<P: AsRef<Path>>(p: &P) -> bool {
    match p.as_ref().parent() {
        Some(parent) => !(!parent.as_os_str().is_empty() && !parent.is_dir()),
        None => true,
    }
}

pub fn checked_dec(x: u64) -> u64 {
    if x > 0 {
        x - 1
    } else {
        x
    }
}

pub fn to_satisfiable_range<T: RangeBounds<u64>>(r: T, len: u64) -> Option<(u64, u64)> {
    match (r.start_bound(), r.end_bound()) {
        (Bound::Included(&start), Bound::Included(&end)) => {
            if start <= end && start < len {
                Some((start, min(end, len - 1)))
            } else {
                None
            }
        }

        (Bound::Included(&start), Bound::Unbounded) => {
            if start < len {
                Some((start, len - 1))
            } else {
                None
            }
        }

        (Bound::Unbounded, Bound::Included(&offset)) => {
            if offset > 0 {
                Some((max(len - offset, 0), len - 1))
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn into_range_bounds(i: (u64, u64)) -> (Bound<u64>, Bound<u64>) {
    (Bound::Included(i.0), Bound::Included(i.1))
}

pub fn header2header<H1: Header, H2: Header>(i: H1) -> Result<impl Header, headers::Error> {
    let mut v = vec![];
    i.encode(&mut v);
    H2::decode(&mut v.iter())
}

struct HeadersExtender<'a, 'b> {
    builder: &'a mut Builder,
    name: &'b HeaderName,
}

impl<'a, 'b> Extend<HeaderValue> for HeadersExtender<'a, 'b> {
    fn extend<I: IntoIterator<Item = HeaderValue>>(&mut self, iter: I) {
        let headers = self.builder.headers_mut().unwrap(); // TODO is it always safe to unwrap()?
        for v in iter.into_iter() {
            headers.insert(self.name, v);
        }
    }
}

pub trait ResponseBuilderExt {
    fn typed_header<H: Header>(&mut self, header: H) -> &mut Builder;
}

impl ResponseBuilderExt for Builder {
    fn typed_header<H: Header>(&mut self, header: H) -> &mut Builder {
        let mut extender = HeadersExtender {
            builder: self,
            name: H::name(),
        };
        header.encode(&mut extender);
        self
    }
}
