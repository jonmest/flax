use memchr::memmem;

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

    pub fn write_ptr_len(&mut self) -> (*mut u8, usize) {
        let free = self.buf.len() - self.end;
        (unsafe { self.buf.as_mut_ptr().add(self.end) }, free)
    }

    pub fn wrote(&mut self, n: usize) {
        self.end += n;
    }

    pub fn find_headers_end(&self) -> Option<usize> {
        let s = &self.buf[self.start..self.end];
        if let Some(pos) = memmem::find(&s, b"\r\n\r\n") {
            return Some(pos + 4);
        }
        None
    }

    pub fn consume_to(&mut self, pos: usize) {
        debug_assert!(pos <= self.end);
        self.start = pos;

        if self.start > self.buf.len() / 2 {
            let len = self.end - self.start;
            self.buf.copy_within(self.start..self.end, 0);
            self.start = 0;
            self.end = len;
        }
    }

    pub fn window(&self) -> &[u8] {
        &self.buf[self.start..self.end]
    }

    pub fn drain(&mut self) -> (Vec<u8>, usize, usize) {
        let buf = std::mem::take(&mut self.buf);
        let (start, end) = (self.start, self.end);
        self.start = 0;
        self.end = 0;
        (buf, start, end)
    }

    pub fn replace_buffer(&mut self, buf: Vec<u8>) {
        self.buf = buf;
        self.start = 0;
        self.end = 0;
    }
}

pub struct HttpMetadata<'a> {
    pub method_bytes: &'a [u8],
    pub path_bytes: &'a [u8],
    pub host_header_value: Option<&'a [u8]>,
    pub content_length_value: Option<usize>,
    pub transfer_encoding_is_chunked: bool,
    pub header_block_end_index: usize,
}

pub fn peek_request_headers<'a>(
    request_window: &'a [u8],
) -> Result<HttpMetadata<'a>, &'static str> {
    let Some(crlf_crlf_position) = memmem::find(request_window, b"\r\n\r\n") else {
        return Err("Incomplete message");
    };

    let header_block_end_index = crlf_crlf_position + 4;
    let headers_without_final_crlfcrlf = &request_window[..crlf_crlf_position];

    let Some(request_line_end_rel) = memmem::find(headers_without_final_crlfcrlf, b"\r\n") else {
        return Err("Bad message");
    };
    let request_line = &headers_without_final_crlfcrlf[..request_line_end_rel];

    let mut fields = request_line.split(|&b| b == b' ');
    let method_bytes = fields.next().ok_or("Bad")?;
    let path_bytes = fields.next().ok_or("Bad")?;
    let version_bytes = fields.next().ok_or("Bad")?;

    if method_bytes.is_empty() || version_bytes.len() < 8 {
        return Err("bad");
    }

    let mut host_header_value: Option<&[u8]> = None;
    let mut content_length_value: Option<usize> = None;
    let mut transfer_encoding_is_chunked = false;

    let mut line_start = request_line_end_rel + 2; // skip CRLF after request-line
    while line_start < headers_without_final_crlfcrlf.len() {
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
