//! Minimal local HTTP client for oMLX embedding benchmarks.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use selene_core::VectorValue;

use super::corpus::CorpusInput;

pub(super) struct OmlxClient {
    endpoint: HttpEndpoint,
    api_key: String,
}

impl OmlxClient {
    pub(super) fn new(base_url: String, api_key: String) -> Self {
        let endpoint = HttpEndpoint::parse(&base_url).expect("valid local oMLX HTTP base URL");
        Self { endpoint, api_key }
    }

    pub(super) fn embed(
        &self,
        model: &str,
        inputs: &[CorpusInput],
    ) -> Result<Vec<VectorValue>, String> {
        let body = serde_json::json!({
            "model": model,
            "input": inputs.iter().map(|input| input.text).collect::<Vec<_>>(),
        })
        .to_string();
        let response = self
            .endpoint
            .post_json("/embeddings", &self.api_key, &body)?;
        let json: serde_json::Value = serde_json::from_slice(&response)
            .map_err(|err| format!("oMLX embedding response is not JSON: {err}"))?;
        let Some(data) = json.get("data").and_then(|value| value.as_array()) else {
            return Err(format!(
                "oMLX embedding response for {model} has no data array: {}",
                truncate_json(&json)
            ));
        };
        data.iter()
            .map(|item| {
                let Some(values) = item.get("embedding").and_then(|value| value.as_array()) else {
                    return Err("oMLX embedding item has no embedding array".to_owned());
                };
                let components = values
                    .iter()
                    .map(|value| {
                        value
                            .as_f64()
                            .map(|component| component as f32)
                            .ok_or_else(|| "oMLX embedding component is not numeric".to_owned())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                VectorValue::new(components)
                    .map_err(|err| format!("oMLX embedding vector failed validation: {err}"))
            })
            .collect()
    }
}

struct HttpEndpoint {
    host: String,
    port: u16,
    path_prefix: String,
}

impl HttpEndpoint {
    fn parse(base_url: &str) -> Result<Self, String> {
        let Some(rest) = base_url.strip_prefix("http://") else {
            return Err("local oMLX bench only supports http:// endpoints".to_owned());
        };
        let (host_port, path) = rest.split_once('/').unwrap_or((rest, ""));
        let (host, port) = match host_port.rsplit_once(':') {
            Some((host, port)) => {
                let port = port
                    .parse::<u16>()
                    .map_err(|err| format!("invalid oMLX port: {err}"))?;
                (host.to_owned(), port)
            }
            None => (host_port.to_owned(), 80),
        };
        if host.is_empty() {
            return Err("local oMLX host must not be empty".to_owned());
        }
        let path_prefix = if path.is_empty() {
            String::new()
        } else {
            format!("/{}", path.trim_end_matches('/'))
        };
        Ok(Self {
            host,
            port,
            path_prefix,
        })
    }

    fn post_json(&self, suffix: &str, api_key: &str, body: &str) -> Result<Vec<u8>, String> {
        let mut stream = TcpStream::connect((self.host.as_str(), self.port))
            .map_err(|err| format!("connect to local oMLX failed: {err}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(120)))
            .map_err(|err| format!("set local oMLX read timeout failed: {err}"))?;
        let path = format!("{}{}", self.path_prefix, suffix);
        let request = format!(
            "POST {path} HTTP/1.1\r\n\
             Host: {host}:{port}\r\n\
             Authorization: Bearer {api_key}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {len}\r\n\
             Connection: close\r\n\
             \r\n\
             {body}",
            host = self.host,
            port = self.port,
            len = body.len(),
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|err| format!("write local oMLX request failed: {err}"))?;
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .map_err(|err| format!("read local oMLX response failed: {err}"))?;
        parse_http_body(&response)
    }
}

fn parse_http_body(response: &[u8]) -> Result<Vec<u8>, String> {
    let Some(header_end) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        return Err("local oMLX response missing HTTP header terminator".to_owned());
    };
    let headers = std::str::from_utf8(&response[..header_end])
        .map_err(|err| format!("local oMLX response header is not UTF-8: {err}"))?;
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| "local oMLX response missing HTTP status".to_owned())?;
    let mut body = response[header_end + 4..].to_vec();
    if has_chunked_transfer_encoding(headers) {
        body = decode_chunked_body(&body)?;
    }
    if !(200..300).contains(&status) {
        return Err(format!(
            "local oMLX request failed with HTTP {status}: {}",
            String::from_utf8_lossy(&body)
                .chars()
                .take(240)
                .collect::<String>()
        ));
    }
    Ok(body)
}

fn has_chunked_transfer_encoding(headers: &str) -> bool {
    headers.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        lower.starts_with("transfer-encoding:") && lower.contains("chunked")
    })
}

fn decode_chunked_body(raw: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoded = Vec::new();
    let mut offset = 0usize;
    loop {
        let line_end = find_crlf(raw, offset)
            .ok_or_else(|| "chunked oMLX response missing chunk-size line end".to_owned())?;
        let size_line = std::str::from_utf8(&raw[offset..line_end])
            .map_err(|err| format!("chunked oMLX size line is not UTF-8: {err}"))?;
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|err| format!("chunked oMLX size is not hex: {err}"))?;
        offset = line_end + 2;
        if size == 0 {
            break;
        }
        let chunk_end = offset
            .checked_add(size)
            .ok_or_else(|| "chunked oMLX body size overflow".to_owned())?;
        if chunk_end + 2 > raw.len() {
            return Err("chunked oMLX body ended before declared chunk size".to_owned());
        }
        decoded.extend_from_slice(&raw[offset..chunk_end]);
        if &raw[chunk_end..chunk_end + 2] != b"\r\n" {
            return Err("chunked oMLX chunk missing trailing CRLF".to_owned());
        }
        offset = chunk_end + 2;
    }
    Ok(decoded)
}

fn find_crlf(bytes: &[u8], start: usize) -> Option<usize> {
    bytes
        .get(start..)?
        .windows(2)
        .position(|window| window == b"\r\n")
        .map(|position| start + position)
}

fn truncate_json(json: &serde_json::Value) -> String {
    json.to_string().chars().take(240).collect()
}
