//! Minimal embedding clients for opt-in benchmark setup.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::Duration;

use selene_core::VectorValue;

use super::corpus::CorpusInput;

/// Embedding provider used by opt-in benchmark helpers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmbeddingProvider {
    /// Local oMLX OpenAI-compatible HTTP endpoint.
    Omlx,
    /// OpenRouter HTTPS embeddings endpoint.
    OpenRouter,
}

impl EmbeddingProvider {
    /// Resolve a provider from `env_name`, using `default` when unset.
    pub fn from_env(env_name: &str, default: Self) -> Self {
        Self::from_value(std::env::var(env_name).ok().as_deref(), default)
    }

    fn from_value(value: Option<&str>, default: Self) -> Self {
        match value {
            None | Some("") => default,
            Some("omlx") => Self::Omlx,
            Some("openrouter") => Self::OpenRouter,
            Some(other) => panic!("unsupported SELENE_EMBEDDING_PROVIDER value: {other}"),
        }
    }
}

/// Embedding client selected by benchmark configuration.
pub enum EmbeddingClient {
    /// Local oMLX client.
    Omlx(OmlxClient),
    /// OpenRouter client.
    OpenRouter(OpenRouterClient),
}

impl EmbeddingClient {
    /// Build a local oMLX embedding client.
    pub fn omlx(base_url: String, api_key: String, batch_size: usize) -> Self {
        Self::Omlx(OmlxClient::new(base_url, api_key, batch_size))
    }

    /// Build an OpenRouter embedding client.
    pub fn openrouter(
        base_url: String,
        api_key: String,
        batch_size: usize,
        referer: Option<String>,
        title: Option<String>,
    ) -> Self {
        Self::OpenRouter(OpenRouterClient::new(
            base_url, api_key, batch_size, referer, title,
        ))
    }

    /// Embed every corpus input with `model`, preserving input order.
    pub fn embed(&self, model: &str, inputs: &[CorpusInput]) -> Result<Vec<VectorValue>, String> {
        match self {
            Self::Omlx(client) => client.embed(model, inputs),
            Self::OpenRouter(client) => client.embed(model, inputs),
        }
    }
}

/// Minimal HTTP client for local oMLX embedding endpoints.
///
/// The client deliberately uses `TcpStream` instead of an HTTP runtime so
/// benchmark crates can opt into local embeddings without pulling in async or
/// networking dependencies.
pub struct OmlxClient {
    endpoint: HttpEndpoint,
    api_key: String,
    batch_size: usize,
}

impl OmlxClient {
    /// Build a client for `base_url`, using `api_key` and chunking requests by
    /// `batch_size`.
    ///
    /// `base_url` must be an `http://` local endpoint. `batch_size` must be
    /// greater than zero.
    pub fn new(base_url: String, api_key: String, batch_size: usize) -> Self {
        let endpoint = HttpEndpoint::parse(&base_url).expect("valid local oMLX HTTP base URL");
        assert!(batch_size > 0, "oMLX embedding batch size must be non-zero");
        Self {
            endpoint,
            api_key,
            batch_size,
        }
    }

    /// Embed every corpus input with `model`, preserving input order.
    pub fn embed(&self, model: &str, inputs: &[CorpusInput]) -> Result<Vec<VectorValue>, String> {
        let mut vectors = Vec::with_capacity(inputs.len());
        for chunk in inputs.chunks(self.batch_size) {
            vectors.extend(self.embed_chunk(model, chunk)?);
        }
        if vectors.len() != inputs.len() {
            return Err(format!(
                "oMLX embedding response for {model} returned {} vectors for {} inputs",
                vectors.len(),
                inputs.len()
            ));
        }
        Ok(vectors)
    }

    fn embed_chunk(&self, model: &str, inputs: &[CorpusInput]) -> Result<Vec<VectorValue>, String> {
        let body = serde_json::json!({
            "model": model,
            "input": inputs.iter().map(CorpusInput::text).collect::<Vec<_>>(),
        })
        .to_string();
        let response = self
            .endpoint
            .post_json("/embeddings", &self.api_key, &body)?;
        parse_embedding_response("oMLX", model, inputs.len(), &response)
    }
}

/// Minimal curl-backed OpenRouter embeddings client.
///
/// This is used only by opt-in local benchmark setup before Criterion timing
/// starts. Using `curl` keeps the benchmark support path dependency-light while
/// still relying on a mature TLS implementation instead of hand-rolled HTTPS.
pub struct OpenRouterClient {
    endpoint: String,
    api_key: String,
    batch_size: usize,
    referer: String,
    title: String,
}

impl OpenRouterClient {
    /// Build an OpenRouter embedding client.
    pub fn new(
        base_url: String,
        api_key: String,
        batch_size: usize,
        referer: Option<String>,
        title: Option<String>,
    ) -> Self {
        assert!(
            batch_size > 0,
            "OpenRouter embedding batch size must be non-zero"
        );
        let endpoint = format!("{}/embeddings", base_url.trim_end_matches('/'));
        Self {
            endpoint,
            api_key,
            batch_size,
            referer: referer
                .unwrap_or_else(|| "https://github.com/jscott3201/selene-db".to_owned()),
            title: title.unwrap_or_else(|| "selene-db local benchmarks".to_owned()),
        }
    }

    /// Embed every corpus input with `model`, preserving input order.
    pub fn embed(&self, model: &str, inputs: &[CorpusInput]) -> Result<Vec<VectorValue>, String> {
        let mut vectors = Vec::with_capacity(inputs.len());
        for chunk in inputs.chunks(self.batch_size) {
            vectors.extend(self.embed_chunk(model, chunk)?);
        }
        if vectors.len() != inputs.len() {
            return Err(format!(
                "OpenRouter embedding response for {model} returned {} vectors for {} inputs",
                vectors.len(),
                inputs.len()
            ));
        }
        Ok(vectors)
    }

    fn embed_chunk(&self, model: &str, inputs: &[CorpusInput]) -> Result<Vec<VectorValue>, String> {
        let body = serde_json::json!({
            "model": model,
            "input": inputs.iter().map(CorpusInput::text).collect::<Vec<_>>(),
            "encoding_format": "float",
        })
        .to_string();
        let response = self.post_json(&body)?;
        parse_embedding_response("OpenRouter", model, inputs.len(), &response)
    }

    fn post_json(&self, body: &str) -> Result<Vec<u8>, String> {
        let script = r#"curl -sS --fail-with-body --connect-timeout 30 --max-time 180 \
  -H "Authorization: Bearer ${OPENROUTER_API_KEY:?}" \
  -H "Content-Type: application/json" \
  -H "HTTP-Referer: ${SELENE_OPENROUTER_HTTP_REFERER:?}" \
  -H "X-OpenRouter-Title: ${SELENE_OPENROUTER_TITLE:?}" \
  --data-binary @- \
  "${SELENE_OPENROUTER_EMBEDDING_URL:?}""#;
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(script)
            .env("OPENROUTER_API_KEY", &self.api_key)
            .env("SELENE_OPENROUTER_HTTP_REFERER", &self.referer)
            .env("SELENE_OPENROUTER_TITLE", &self.title)
            .env("SELENE_OPENROUTER_EMBEDDING_URL", &self.endpoint)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("spawn OpenRouter curl request failed: {err}"))?;
        child
            .stdin
            .as_mut()
            .ok_or_else(|| "OpenRouter curl stdin was not piped".to_owned())?
            .write_all(body.as_bytes())
            .map_err(|err| format!("write OpenRouter request body failed: {err}"))?;
        let output = child
            .wait_with_output()
            .map_err(|err| format!("wait for OpenRouter curl request failed: {err}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let detail = if stdout.trim().is_empty() {
                stderr.as_ref()
            } else {
                stdout.as_ref()
            };
            return Err(format!(
                "OpenRouter embedding request failed with curl status {}: {}",
                output
                    .status
                    .code()
                    .map_or_else(|| "signal".to_owned(), |code| code.to_string()),
                detail.chars().take(240).collect::<String>()
            ));
        }
        Ok(output.stdout)
    }
}

fn parse_embedding_response(
    source: &str,
    model: &str,
    expected_count: usize,
    response: &[u8],
) -> Result<Vec<VectorValue>, String> {
    let json: serde_json::Value = serde_json::from_slice(response)
        .map_err(|err| format!("{source} embedding response is not JSON: {err}"))?;
    let Some(data) = json.get("data").and_then(|value| value.as_array()) else {
        return Err(format!(
            "{source} embedding response for {model} has no data array: {}",
            truncate_json(&json)
        ));
    };
    let vectors = data
        .iter()
        .map(|item| {
            let Some(values) = item.get("embedding").and_then(|value| value.as_array()) else {
                return Err(format!("{source} embedding item has no embedding array"));
            };
            let components = values
                .iter()
                .map(|value| {
                    value
                        .as_f64()
                        .map(|component| component as f32)
                        .ok_or_else(|| format!("{source} embedding component is not numeric"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            VectorValue::new(components)
                .map_err(|err| format!("{source} embedding vector failed validation: {err}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if vectors.len() != expected_count {
        return Err(format!(
            "{source} embedding response for {model} returned {} vectors for {expected_count} inputs",
            vectors.len()
        ));
    }
    Ok(vectors)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_style_embedding_response() {
        let body = br#"{
            "data": [
                {"embedding": [0.25, -1.5]},
                {"embedding": [2.0, 3.5]}
            ]
        }"#;

        let vectors = parse_embedding_response("test", "model", 2, body).expect("response parses");

        assert_eq!(vectors.len(), 2);
        assert_eq!(vectors[0].dimension(), 2);
        assert_eq!(vectors[0].as_slice(), &[0.25, -1.5]);
        assert_eq!(vectors[1].as_slice(), &[2.0, 3.5]);
    }

    #[test]
    fn rejects_embedding_response_count_mismatch() {
        let body = br#"{"data": [{"embedding": [1.0]}]}"#;

        let err = parse_embedding_response("test", "model", 2, body).unwrap_err();

        assert!(err.contains("returned 1 vectors for 2 inputs"));
    }

    #[test]
    fn provider_value_defaults_when_unset() {
        assert_eq!(
            EmbeddingProvider::from_value(None, EmbeddingProvider::OpenRouter),
            EmbeddingProvider::OpenRouter
        );
        assert_eq!(
            EmbeddingProvider::from_value(Some(""), EmbeddingProvider::Omlx),
            EmbeddingProvider::Omlx
        );
    }

    #[test]
    fn provider_value_honors_explicit_selection() {
        assert_eq!(
            EmbeddingProvider::from_value(Some("openrouter"), EmbeddingProvider::Omlx),
            EmbeddingProvider::OpenRouter
        );
        assert_eq!(
            EmbeddingProvider::from_value(Some("omlx"), EmbeddingProvider::OpenRouter),
            EmbeddingProvider::Omlx
        );
    }
}
