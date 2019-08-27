//! ### Note: not stable APIS
//! The items exported in this module are not considered part of this crate's public API
//! and may receive breaking changes in semver-compatible versions.
use futures::{stream, Stream, StreamExt};
use futures::Poll::*;
use futures_test::stream::StreamTestExt;
use futures_test::task::noop_context;

use crate::test_util::BOUNDARY;

use std::cmp;

pub use crate::server::fuzzing::*;

pub use crate::StringError;
use crate::helpers::show_bytes;

/// Deterministically chunk test data so the fuzzer can discover new code paths
pub fn chunk_fuzz_data<'d>(data: &'d [u8]) -> impl Stream<Item = Result<&'d [u8], StringError>> + 'd {
    // this ensures the test boundary will always be split between chunks
    stream::iter(data.chunks(BOUNDARY.len() - 1))
        .map(Ok)
        .interleave_pending()
}

pub fn fuzz_boundary_finder(fuzz_data: &[u8]) {
    let finder = BoundaryFinder::new(chunk_fuzz_data(fuzz_data), BOUNDARY);
    pin_mut!(finder);

    let ref mut cx = noop_context();

    loop {
        match finder.as_mut().consume_boundary(cx) {
            Ready(Ok(false)) | Ready(Err(_)) => return,
            Ready(Ok(true)) => (),
            Pending => continue,
        }

        loop {
            match finder.as_mut().body_chunk(cx) {
                Ready(Some(Ok(chunk))) => {
                    assert_ne!(chunk, &[]);
                    assert_eq!(twoway::find_bytes(chunk, BOUNDARY.as_bytes()), None)
                },
                Pending => (),
                Ready(None) | Ready(Some(Err(_))) => return,
            }
        }
    }
}

/// Fuzz BoundaryFinder taking the input as the data of a field
pub fn fuzz_boundary_finder_field(fuzz_data: &[u8]) {
    // ensure the boundary doesn't appear in the input data
    if twoway::find_bytes(fuzz_data, BOUNDARY.as_bytes()).is_some() { return; }

    let start = format!("{}\r\n", BOUNDARY);
    let end = format!("\r\n{}--", BOUNDARY);
    let stream = chunk_fuzz_data(start.as_bytes())
        .chain(chunk_fuzz_data(fuzz_data))
        .chain(chunk_fuzz_data(end.as_bytes()));

    let finder = BoundaryFinder::new(stream, BOUNDARY);
    pin_mut!(finder);

    let ref mut cx = noop_context();

    loop {
        match finder.as_mut().consume_boundary(cx) {
            Ready(Ok(true)) => {
                break
            },
            Ready(Ok(false)) => panic!("failed to read starting boundary"),
            // errors mean we handled the problem correctly
            Ready(Err(_)) => return,
            Pending => (),
        }
    }

    let mut remaining = fuzz_data;

    loop {
        match finder.as_mut().body_chunk(cx) {
            Ready(Some(Ok(chunk))) => {
                assert_ne!(chunk, &[]);
                assert!(
                    remaining.starts_with(chunk),
                    "expected chunk \"{}\" to be a prefix of remaining data \"{}\"",
                    show_bytes(chunk), show_bytes(remaining)
                );
                remaining = &remaining[chunk.len()..];
            },
            Ready(Some(Err(_))) => return,
            Ready(None) => {
                assert_eq!(remaining, &[]);
                break;
            },
            Pending => (),
        }
    }

    loop {
        match finder.as_mut().consume_boundary(cx) {
            Ready(Ok(false)) => {
                break
            },
            Ready(Ok(true)) => panic!("didn't find ending boundary"),
            Ready(Err(_)) => return,
            Pending => (),
        }
    }
}

pub fn fuzz_read_headers(fuzz_data: &[u8]) {
    if twoway::find_bytes(fuzz_data, BOUNDARY.as_bytes()).is_some() { return }

    let finder = BoundaryFinder::new(chunk_fuzz_data(fuzz_data), BOUNDARY);
    pin_mut!(finder);

    let ref mut cx = noop_context();
    let mut read_headers = ReadHeaders::default();

    while let Pending = read_headers.read_headers(finder.as_mut(), cx) { }
}

#[test]
fn test_fuzz_boundary_finder() {
    let _ = env_logger::try_init();
    fuzz_boundary_finder(b"--boundary\r\n");
}

#[test]
fn test_fuzz_boundary_finder_field() {
    let _ = env_logger::try_init();
    fuzz_boundary_finder_field(b"\r");
    fuzz_boundary_finder_field(b"\r\n--boundar");
    fuzz_boundary_finder_field(b"asdf1234ghjk5678zxcvnm90-=`023458nsdzfdl-");
}

#[test]
fn test_fuzz_read_headers() {
    let _ = env_logger::try_init();
    fuzz_read_headers(b"");
    fuzz_read_headers(b"Content-Disposition: multipart/form-data; name=foo");
}