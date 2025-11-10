use memchr::memmem;

/// HTTP/1.1 Protocol Implementation
///
/// This module provides zero-copy HTTP/1.1 header parsing and buffering
/// for the load balancer. It includes:
/// - HttpBuf: Ring buffer for incoming HTTP headers
/// - HttpMetadata: Parsed request metadata (method, path, headers)
/// - Parsing functions for extracting HTTP information

// ============================================================================
// HTTP Buffer Management
// ============================================================================

/// Ring buffer for HTTP headers with zero-copy operations
pub struct HttpBuf {
    buf: Vec<u8>,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

impl HttpBuf {
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf: vec![0; cap],
            start: 0,
            end: 0,
        }
    }

    /// Get pointer and length for the next recv operation
    pub fn write_ptr_len(&mut self) -> (*mut u8, usize) {
        let free = self.buf.len() - self.end;
        (unsafe { self.buf.as_mut_ptr().add(self.end) }, free)
    }

    /// Mark that n bytes were written to the buffer
    pub fn wrote(&mut self, n: usize) {
        self.end += n;
    }

    /// Search for \r\n\r\n in current window (header end marker)
    pub fn find_headers_end(&self) -> Option<usize> {
        let s = &self.buf[self.start..self.end];
        if let Some(pos) = memmem::find(&s, b"\r\n\r\n") {
            return Some(pos + 4);
        }
        None
    }

    /// Consume bytes up to position (advance start pointer)
    /// Compacts the buffer if start passes the halfway point
    pub fn consume_to(&mut self, pos: usize) {
        debug_assert!(pos <= self.end);
        self.start = pos;

        // Compact buffer if we've consumed more than half
        if self.start > self.buf.len() / 2 {
            let len = self.end - self.start;
            self.buf.copy_within(self.start..self.end, 0);
            self.start = 0;
            self.end = len;
        }
    }

    /// Get the current readable window
    pub fn window(&self) -> &[u8] {
        &self.buf[self.start..self.end]
    }
}

// ============================================================================
// HTTP Metadata Parsing
// ============================================================================

/// Parsed HTTP request metadata (zero-copy - references input buffer)
pub struct HttpMetadata<'a> {
    pub method_bytes: &'a [u8],
    pub path_bytes: &'a [u8],
    pub host_header_value: Option<&'a [u8]>,
    pub content_length_value: Option<usize>,
    pub transfer_encoding_is_chunked: bool,
    pub header_block_end_index: usize,
}

/// Parse HTTP request headers from a buffer
///
/// This function performs zero-copy parsing of HTTP/1.1 request headers.
/// It extracts:
/// - Request line (method, path, version)
/// - Host header (for routing)
/// - Content-Length header (for body handling)
/// - Transfer-Encoding: chunked detection
///
/// Returns Err("Incomplete message") if headers are not yet fully received
pub fn peek_request_headers<'a>(request_window: &'a [u8]) -> Result<HttpMetadata<'a>, &'static str> {
    // 1. Find the end of header block (\r\n\r\n)
    let Some(crlf_crlf_position) = memmem::find(request_window, b"\r\n\r\n") else {
        return Err("Incomplete message");
    };

    let header_block_end_index = crlf_crlf_position + 4;
    let headers_without_final_crlfcrlf = &request_window[..crlf_crlf_position];

    // 2. Extract request line
    let Some(request_line_end_rel) = memmem::find(headers_without_final_crlfcrlf, b"\r\n") else {
        return Err("Bad message");
    };
    let request_line = &headers_without_final_crlfcrlf[..request_line_end_rel];

    // Parse method, path, and version
    let mut fields = request_line.split(|&b| b == b' ');
    let method_bytes = fields.next().ok_or("Bad")?;
    let path_bytes = fields.next().ok_or("Bad")?;
    let version_bytes = fields.next().ok_or("Bad")?;

    eprintln!("[DEBUG PARSE] method={:?}, path={:?}, version={:?}",
              String::from_utf8_lossy(method_bytes),
              String::from_utf8_lossy(path_bytes),
              String::from_utf8_lossy(version_bytes));
    eprintln!("[DEBUG PARSE] method.len()={}, path.len()={}, version.len()={}",
              method_bytes.len(), path_bytes.len(), version_bytes.len());

    if method_bytes.is_empty() || version_bytes.len() < 8 {
        eprintln!("[DEBUG PARSE] Validation failed: method.is_empty()={}, version.len() < 8 = {}",
                  method_bytes.is_empty(), version_bytes.len() < 8);
        return Err("bad");
    }

    // 3. Parse interesting header lines
    let mut host_header_value: Option<&[u8]> = None;
    let mut content_length_value: Option<usize> = None;
    let mut transfer_encoding_is_chunked = false;

    let mut line_start = request_line_end_rel + 2; // skip CRLF after request-line
    eprintln!("[DEBUG PARSE] Starting header loop, line_start={}, headers_len={}",
              line_start, headers_without_final_crlfcrlf.len());
    while line_start < headers_without_final_crlfcrlf.len() {
        eprintln!("[DEBUG PARSE] Looking for next header line at offset {}", line_start);
        // Find the end of this header line
        let line_end = if let Some(rel_line_end) =
            memmem::find(&headers_without_final_crlfcrlf[line_start..], b"\r\n")
        {
            line_start + rel_line_end
        } else {
            // No more CRLF found - this is the last header line
            headers_without_final_crlfcrlf.len()
        };
        let header_line = &headers_without_final_crlfcrlf[line_start..line_end];
        line_start = line_end + 2; // advance past CRLF

        if let Some(colon_index) = header_line.iter().position(|&b| b == b':') {
            let (raw_name, raw_value) = header_line.split_at(colon_index);
            let value = trim_ascii_whitespace(&raw_value[1..]); // skip ':'

            if ascii_equals_ignore_case(raw_name, b"Host") {
                if !value.is_empty() {
                    host_header_value = Some(value);
                }
            } else if ascii_equals_ignore_case(raw_name, b"Content-Length") {
                content_length_value = parse_usize_decimal_strict(value);
            } else if ascii_equals_ignore_case(raw_name, b"Transfer-Encoding") {
                // check comma-separated tokens for "chunked"
                for token in value.split(|&b| b == b',') {
                    if ascii_equals_ignore_case(trim_ascii_whitespace(token), b"chunked") {
                        transfer_encoding_is_chunked = true;
                        break;
                    }
                }
            }
        }
    }

    Ok(HttpMetadata {
        method_bytes,
        path_bytes,
        host_header_value,
        content_length_value,
        transfer_encoding_is_chunked,
        header_block_end_index,
    })
}

// ============================================================================
// Helper Functions
// ============================================================================

#[inline]
fn ascii_equals_ignore_case(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).all(|(&x, &y)| x.eq_ignore_ascii_case(&y))
}

#[inline]
fn trim_ascii_whitespace(mut bytes: &[u8]) -> &[u8] {
    while !bytes.is_empty() && (bytes[0] == b' ' || bytes[0] == b'\t') {
        bytes = &bytes[1..];
    }
    while !bytes.is_empty() && (bytes[bytes.len() - 1] == b' ' || bytes[bytes.len() - 1] == b'\t') {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

#[inline]
fn parse_usize_decimal_strict(input: &[u8]) -> Option<usize> {
    let mut value: usize = 0;
    for &ch in trim_ascii_whitespace(input) {
        if !(b'0'..=b'9').contains(&ch) {
            return None;
        }
        let digit = (ch - b'0') as usize;
        value = value.checked_mul(10)?.checked_add(digit)?;
    }
    Some(value)
}

// ============================================================================
// Legacy Types (for compatibility - may be removed later)
// ============================================================================

#[derive(Debug)]
pub struct RequestLine<'a> {
    pub(crate) method: &'a [u8],
    pub(crate) path: &'a [u8],
    pub(crate) version: &'a [u8],
}

pub fn parse_request_line<'a>(buf: &'a [u8]) -> Result<(usize, RequestLine<'a>), &'static str> {
    let line_end = memmem::find(&buf, b"\r\n");
    match line_end {
        Some(line_end) => {
            let line = &buf[..line_end];
            let mut it = line.split(|&b| b == b' ');
            let method = it.next().ok_or("bad reqline")?;
            let path = it.next().ok_or("bad reqline")?;
            let version = it.next().ok_or("bad reqline")?;

            if method.is_empty() || version.len() < 8 {
                return Err("bad reqline");
            }

            Ok((
                line_end + 2,
                RequestLine {
                    method,
                    path,
                    version,
                },
            ))
        }
        None => Err("Incomplete request"),
    }
}
